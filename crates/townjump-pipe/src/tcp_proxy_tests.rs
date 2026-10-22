// Tests of `create_tcp_proxy`: accepting on a local port, `connect` per connection, pumping (pipe.md, `createTcpProxy`).
// The remote side is a `duplex` from `remotes()`; `setup` and helpers are at the bottom of the file.
use super::*;
use crate::support::*;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

mod listening {
    use super::*;

    #[tokio::test]
    async fn port_accepts_connections_right_after_return() {
        let p = setup(Opts::default()).await;
        assert_ne!(
            p.proxy.local_addr.port(),
            0,
            "port 0 — the OS picks, local_addr holds the real one"
        );
        assert!(p.proxy.local_addr.ip().is_loopback());
        let _client = connect_client(p.proxy.local_addr).await;
    }

    #[tokio::test]
    async fn busy_port_gives_an_addr_in_use_error() {
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let (_remotes, connect) = remotes(0);
        let listen = ("127.0.0.1".to_owned(), port);
        let result = create_tcp_proxy(listen, connect, PiperOpts::default(), Arc::default()).await;
        let err = result.expect_err("port busy — bind must give Err");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        // The user-facing text is built by the CLI (townjump-client) from the `ErrorKind`; the OS
        // text differs per platform ("address already in use" vs Windows os error 10048).
    }

    #[tokio::test]
    async fn abort_stops_accepting_but_accepted_connections_keep_living() {
        let mut p = setup(Opts::default()).await;
        let addr = p.proxy.local_addr;
        let mut client = connect_client(addr).await;
        let mut far = p.remotes.next().await;
        p.proxy.task.abort();
        let _ = within("accept task stopped", &mut p.proxy.task).await;
        client.write_all(b"live").await.unwrap();
        assert_eq!(
            read_n(&mut far, 4).await,
            b"live",
            "an accepted connection must stay alive"
        );
        let late = within("new connection", TcpStream::connect(addr)).await;
        assert!(late.is_err(), "after abort the port no longer accepts");
    }
}

mod per_connection_connect {
    use super::*;

    #[tokio::test]
    async fn connect_is_not_called_until_a_client_arrives() {
        let mut p = setup(Opts::default()).await;
        p.remotes.none_new().await;
        assert_eq!(p.remotes.calls(), 0);
    }

    #[tokio::test]
    async fn connect_is_called_at_once_for_a_client_that_sent_nothing() {
        let mut p = setup(Opts::default()).await;
        let _client = connect_client(p.proxy.local_addr).await;
        let _far = p.remotes.next().await;
    }

    #[tokio::test]
    async fn connect_is_called_once_per_accepted_connection() {
        let mut p = setup(Opts::default()).await;
        let mut clients = Vec::new();
        for _ in 0..3 {
            clients.push(connect_client(p.proxy.local_addr).await);
            let _far = p.remotes.next().await;
        }
        assert_eq!(p.remotes.calls(), 3);
    }

    #[tokio::test]
    async fn each_connection_talks_to_its_own_remote_stream() {
        let mut p = setup(Opts::default()).await;
        let mut first = connect_client(p.proxy.local_addr).await;
        let mut far_first = p.remotes.next().await;
        let mut second = connect_client(p.proxy.local_addr).await;
        let mut far_second = p.remotes.next().await;
        first.write_all(b"from-first").await.unwrap();
        second.write_all(b"from-second").await.unwrap();
        assert_eq!(read_n(&mut far_first, 10).await, b"from-first");
        assert_eq!(read_n(&mut far_second, 11).await, b"from-second");
        far_second.write_all(b"to-second").await.unwrap();
        assert_eq!(read_n(&mut second, 9).await, b"to-second");
        let mut byte = [0u8; 1];
        stays_pending("2nd reply reached the 1st", first.read(&mut byte)).await;
    }
}

mod data_flow {
    use super::*;

    #[tokio::test]
    async fn a_quarter_megabyte_goes_each_way_without_loss() {
        let mut p = setup(Opts::default()).await;
        let mut client = connect_client(p.proxy.local_addr).await;
        let mut far = p.remotes.next().await;
        let (up, down) = (pattern(256 * 1024, 1), pattern(256 * 1024, 2));
        let (mut client_rd, mut client_wr) = client.split();
        let (mut far_rd, mut far_wr) = tokio::io::split(&mut far);
        let (sent_up, sent_down, got_up, got_down) = tokio::join!(
            client_wr.write_all(&up),
            far_wr.write_all(&down),
            read_n(&mut far_rd, up.len()),
            read_n(&mut client_rd, down.len())
        );
        sent_up.unwrap();
        sent_down.unwrap();
        assert!(
            got_up == up,
            "the remote side received something other than what the client wrote"
        );
        assert!(
            got_down == down,
            "the client received something other than what the remote side wrote"
        );
    }

    #[tokio::test]
    async fn client_end_reaches_remote_and_the_answer_still_comes_back() {
        let mut p = setup(Opts::default()).await;
        let mut client = connect_client(p.proxy.local_addr).await;
        let mut far = p.remotes.next().await;
        client.shutdown().await.unwrap();
        expect_eof(&mut far, "remote side after the client's end").await;
        far.write_all(b"tail").await.unwrap();
        assert_eq!(
            read_n(&mut client, 4).await,
            b"tail",
            "allowHalfOpen: the response comes after the client's shutdown"
        );
    }

    #[tokio::test]
    async fn remote_end_reaches_client_and_the_client_can_still_send() {
        let mut p = setup(Opts::default()).await;
        let mut client = connect_client(p.proxy.local_addr).await;
        let mut far = p.remotes.next().await;
        far.shutdown().await.unwrap();
        expect_eof(&mut client, "client after the remote side's end").await;
        client.write_all(b"late").await.unwrap();
        assert_eq!(read_n(&mut far, 4).await, b"late");
    }
}

mod connect_fails {
    use super::*;

    #[tokio::test]
    async fn that_client_is_closed() {
        let p = setup(Opts {
            connect_failures: 1,
            ..Opts::default()
        })
        .await;
        let mut client = connect_client(p.proxy.local_addr).await;
        expect_closed(&mut client, "client on connect error").await;
    }

    #[tokio::test]
    async fn the_next_client_is_served_normally() {
        let mut p = setup(Opts {
            connect_failures: 1,
            ..Opts::default()
        })
        .await;
        let mut failed = connect_client(p.proxy.local_addr).await;
        expect_closed(&mut failed, "first client").await;
        let mut client = connect_client(p.proxy.local_addr).await;
        let mut far = p.remotes.next().await;
        client.write_all(b"ok").await.unwrap();
        assert_eq!(read_n(&mut far, 2).await, b"ok");
        assert_eq!(p.remotes.calls(), 2);
    }

    #[tokio::test]
    async fn counters_do_not_keep_the_failed_connection() {
        let p = setup(Opts {
            connect_failures: 1,
            ..Opts::default()
        })
        .await;
        let mut client = connect_client(p.proxy.local_addr).await;
        expect_closed(&mut client, "client on connect error").await;
        wait_until("counters returned to zero", || {
            p.watch.loc_rem_cnt() == (0, 0)
        })
        .await;
    }
}

mod many_connections {
    use super::*;

    #[tokio::test]
    async fn fifty_parallel_clients_each_get_their_own_echo() {
        let watch = Watch::new(false);
        let listen = ("127.0.0.1".to_owned(), 0);
        let proxy =
            create_tcp_proxy(listen, echoing_remote(), watch.opts(), watch.stats.clone()).await;
        let addr = proxy.expect("bind").local_addr;
        let mut clients = JoinSet::new();
        for i in 0..50 {
            clients.spawn(echo_round_trip(addr, i));
        }
        let all_done = async {
            while let Some(done) = clients.join_next().await {
                done.expect("client panicked");
            }
        };
        within("50 clients", all_done).await;
    }

    #[tokio::test]
    async fn live_connections_are_counted_and_released() {
        let watch = Watch::new(false);
        let listen = ("127.0.0.1".to_owned(), 0);
        let proxy =
            create_tcp_proxy(listen, echoing_remote(), watch.opts(), watch.stats.clone()).await;
        let addr = proxy.expect("bind").local_addr;
        let mut clients = Vec::new();
        for i in 0..3 {
            let mut client = connect_client(addr).await;
            client.write_all(&[i]).await.unwrap();
            assert_eq!(read_n(&mut client, 1).await, [i]);
            clients.push(client);
        }
        assert_eq!(watch.loc_rem_cnt(), (3, 3));
        drop(clients);
        wait_until("counters returned to zero", || {
            watch.loc_rem_cnt() == (0, 0)
        })
        .await;
    }
}

mod log {
    use super::*;

    #[tokio::test]
    async fn every_accepted_connection_logs_starting_the_piper() {
        let mut p = setup(Opts {
            logging: true,
            ..Opts::default()
        })
        .await;
        let _first = connect_client(p.proxy.local_addr).await;
        let _second = connect_client(p.proxy.local_addr).await;
        let _far = (p.remotes.next().await, p.remotes.next().await);
        let starts = || {
            p.watch
                .log
                .positions("INFO", "Starting TCP connection piper")
                .len()
        };
        wait_until("two log lines Starting TCP connection piper", || {
            starts() == 2
        })
        .await;
    }

    #[tokio::test]
    async fn remote_stream_bytes_are_logged_as_local_to_remote_like_in_js() {
        // Naming quirk (pipe.md): on the client the "local/remote" labels are reversed — `loc` here is the DHT stream, not the local socket.
        // JS (testvectors/probes-js/pipe.cjs, `tcpProxy_directions`): the same lines; they are deliberately repeated 1:1.
        let mut p = setup(Opts {
            logging: true,
            ..Opts::default()
        })
        .await;
        let mut client = connect_client(p.proxy.local_addr).await;
        let mut far = p.remotes.next().await;
        far.write_all(b"answer").await.unwrap();
        p.watch
            .log
            .wait_has("DEBUG", "Data from local to remote: 6 bytes")
            .await;
        client.write_all(b"abcd").await.unwrap();
        p.watch
            .log
            .wait_has("DEBUG", "Data from remote to local: 4 bytes")
            .await;
    }
}

#[derive(Default)]
struct Opts {
    connect_failures: usize,
    logging: bool,
}

struct Proxy {
    proxy: TcpProxy,
    remotes: Remotes,
    watch: Watch,
}

/// Proxy on `127.0.0.1:0`; `connect` gives a `duplex`, the first `connect_failures` calls are refused.
async fn setup(opts: Opts) -> Proxy {
    let (remotes, connect) = remotes(opts.connect_failures);
    let watch = Watch::new(opts.logging);
    let listen = ("127.0.0.1".to_owned(), 0);
    let proxy = create_tcp_proxy(listen, connect, watch.opts(), watch.stats.clone()).await;
    Proxy {
        proxy: proxy.expect("bind on loopback"),
        remotes,
        watch,
    }
}

async fn connect_client(addr: SocketAddr) -> TcpStream {
    within("connecting to the proxy", TcpStream::connect(addr))
        .await
        .expect("connect to the proxy")
}

/// A client sends its own set of bytes through the echo proxy and gets it back.
async fn echo_round_trip(addr: SocketAddr, i: u32) {
    let mut client = connect_client(addr).await;
    let data = pattern(2000 + i as usize * 37, i + 1);
    client.write_all(&data).await.unwrap();
    assert!(
        read_n(&mut client, data.len()).await == data,
        "client {i} received foreign bytes"
    );
    client.shutdown().await.unwrap();
}
