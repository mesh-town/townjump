//! Tests for [`super`]: moving a server stream to the socket that got the client's first frame.
//!
//! `UdxStream::write` resolves only after an ACK, so every write here runs in its own task and the
//! test looks at what the peer socket receives instead. The peer is a bare socket with `recv_start`:
//! frames of streams it does not know land in its fallback lane.

use super::*;
use libudx::{Datagram, UdxRuntime};

const ID: u64 = 7;
const REMOTE_ID: u32 = 4242;
const WAIT: Duration = Duration::from_secs(2);

mod adopt_moved_socket {
    use super::*;

    mod without_a_moved_socket {
        use super::*;

        #[tokio::test]
        async fn keeps_the_original() {
            // given
            let f = setup(Opts { armed: true }).await;
            let (a, p_addr) = (f.a.clone(), f.peer.addr);

            // when
            let got = adopt_moved_socket(&f.stream, a.clone(), None, REMOTE_ID, p_addr).await;

            // then
            assert!(
                got.ptr_eq(&a),
                "no moved socket: the original must come back"
            );
        }
    }

    mod with_a_moved_socket_and_an_armed_stream {
        use super::*;

        #[tokio::test]
        async fn takes_over_the_stream() {
            // given
            let f = setup(Opts { armed: true }).await;
            let (a, b, p_addr) = (f.a.clone(), f.b.clone(), f.peer.addr);

            // when
            let got = adopt_moved_socket(&f.stream, a, Some(b.clone()), REMOTE_ID, p_addr).await;

            // then
            assert!(got.ptr_eq(&b), "the moved socket must come back");
            assert_eq!(f.then_write_leaves_from().await, port_of(&b).await);
        }
    }

    mod with_a_moved_socket_and_a_connected_stream {
        use super::*;

        #[tokio::test]
        async fn keeps_the_original() {
            // given
            let f = setup(Opts { armed: false }).await;
            let (a, b, p_addr) = (f.a.clone(), f.b.clone(), f.peer.addr);
            f.stream
                .connect(&a, REMOTE_ID, p_addr)
                .await
                .expect("connect");

            // when
            let got = adopt_moved_socket(&f.stream, a.clone(), Some(b), REMOTE_ID, p_addr).await;

            // then
            assert!(
                got.ptr_eq(&a),
                "connect elsewhere fails: the original must come back"
            );
            assert_eq!(f.then_write_leaves_from().await, port_of(&a).await);
        }
    }
}

mod follow_frames {
    use super::*;

    mod two_frames_in_the_channel {
        use super::*;

        #[tokio::test]
        async fn first_frame_fills_the_cell_then_reports_arrival() {
            // given
            let rt = UdxRuntime::new().expect("runtime");
            let (b, c) = (bound(&rt).await, bound(&rt).await);
            let (addr_b, addr_c) = (addr("10.0.0.1:1111"), addr("10.0.0.2:2222"));
            let (itx, mut irx) = mpsc::unbounded_channel();
            let cell = FollowStream::new(ID, itx);
            let (ftx, frx) = mpsc::unbounded_channel();
            ftx.send(UdxFrameSeen {
                socket: b.clone(),
                addr: addr_b,
            })
            .expect("send b");
            ftx.send(UdxFrameSeen {
                socket: c,
                addr: addr_c,
            })
            .expect("send c");

            // when
            timeout(WAIT, follow_frames(cell.clone(), frx))
                .await
                .expect("follow_frames returns");

            // then
            let held = cell.moved_socket().expect("cell holds a socket");
            assert!(
                held.ptr_eq(&b),
                "the first frame's socket must be in the cell"
            );
            let Some(Internal::Arrived { id, addr }) = irx.try_recv().ok() else {
                panic!("expected Arrived");
            };
            assert_eq!((id, addr), (ID, addr_b));
            assert!(irx.try_recv().is_err(), "only one Arrived for two frames");
        }
    }

    mod channel_closed_without_frames {
        use super::*;

        #[tokio::test]
        async fn reports_nothing() {
            // given
            let (itx, mut irx) = mpsc::unbounded_channel();
            let cell = FollowStream::new(ID, itx);
            let (ftx, frx) = mpsc::unbounded_channel();
            drop(ftx);

            // when
            timeout(WAIT, follow_frames(cell.clone(), frx))
                .await
                .expect("follow_frames returns");

            // then
            assert!(irx.try_recv().is_err(), "no frame, no Arrived");
            assert!(cell.moved_socket().is_none(), "cell must stay empty");
        }
    }
}

/// A stream armed on socket `a`, a second socket `b`, and a peer `P` the stream will be pointed at.
struct Fixture {
    _rt: UdxRuntime,
    a: UdxSocket,
    b: UdxSocket,
    stream: UdxStream,
    peer: Peer,
}

impl Fixture {
    /// Writes on the stream (in a task: the write waits for an ACK that never comes) and returns the
    /// source port of the frame the peer got.
    async fn then_write_leaves_from(mut self) -> u16 {
        let stream = self.stream;
        tokio::spawn(async move {
            let _ = stream.write(b"x").await;
        });
        let dgram = timeout(WAIT, self.peer.rx.recv())
            .await
            .expect("peer got no frame in 2 s")
            .expect("peer lane closed");
        dgram.addr.port()
    }
}

struct Peer {
    addr: SocketAddr,
    rx: mpsc::UnboundedReceiver<Datagram>,
    _socket: UdxSocket,
}

/// `armed`: the stream has the client-first firewall hook on socket `a` (the server before the first frame).
struct Opts {
    armed: bool,
}

async fn setup(opts: Opts) -> Fixture {
    let rt = UdxRuntime::new().expect("runtime");
    let (a, b) = (bound(&rt).await, bound(&rt).await);
    let peer_socket = bound(&rt).await;
    let rx = peer_socket.recv_start().expect("peer recv_start");
    let peer = Peer {
        addr: local(&peer_socket).await,
        rx,
        _socket: peer_socket,
    };
    let stream = rt
        .create_stream(crate::hyperdht::alloc_stream_id())
        .await
        .expect("stream");
    let (itx, _irx) = mpsc::unbounded_channel();
    if opts.armed {
        arm_hook(&stream, &a, REMOTE_ID, ID, &itx).expect("arm_hook");
    }
    Fixture {
        _rt: rt,
        a,
        b,
        stream,
        peer,
    }
}

async fn bound(rt: &UdxRuntime) -> UdxSocket {
    let socket = rt.create_socket().await.expect("socket");
    socket.bind(addr("127.0.0.1:0")).await.expect("bind");
    socket
}

async fn local(socket: &UdxSocket) -> SocketAddr {
    socket.local_addr().await.expect("local_addr")
}

async fn port_of(socket: &UdxSocket) -> u16 {
    local(socket).await.port()
}

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("socket address")
}
