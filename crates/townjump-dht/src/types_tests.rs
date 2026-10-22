// Tests of the `types` module: createServer/connect options, mutableGet record and `DhtConnection` — the shapes that reach the JS trace.
// Trace (cli-matrix.json): `firewall` is a function only in private mode, `reusableSocket: true`, UDP `connect` without `opts`.
use super::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod server_opts {
    use super::*;

    #[test]
    fn default_has_no_firewall_so_everyone_may_connect() {
        let opts = ServerOpts::default();

        assert!(
            opts.firewall.is_none(),
            "None = let everyone in, like `firewall: false` in JS"
        );
    }

    #[test]
    fn default_has_no_relay() {
        assert_eq!(ServerOpts::default().relay_through, None);
    }

    #[test]
    fn default_does_not_reuse_the_socket() {
        let opts = ServerOpts::default();

        assert!(
            !opts.reusable_socket,
            "`reusableSocket: true` is set explicitly by the caller"
        );
    }

    #[test]
    fn firewall_gets_the_client_key_and_answer_true_means_refuse() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let refusing = with_firewall(recording_firewall(&seen, true));
        let allowing = with_firewall(recording_firewall(&seen, false));

        assert!(ask(&refusing, &[7; 32]), "true — reject");
        assert!(!ask(&allowing, &[8; 32]), "false — let through");

        assert_eq!(*seen.lock().unwrap(), vec![[7; 32], [8; 32]]);
    }

    #[test]
    fn clone_shares_the_firewall_callback_and_keeps_the_flag() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let original = ServerOpts {
            firewall: Some(Arc::new(move |_: &[u8; 32]| {
                counter.fetch_add(1, Ordering::SeqCst);
                false
            })),
            reusable_socket: true,
            relay_through: None,
        };

        let copy = original.clone();
        ask(&original, &[1; 32]);
        ask(&copy, &[2; 32]);

        assert_eq!(calls.load(Ordering::SeqCst), 2, "clone calls same callback");
        assert!(copy.reusable_socket, "the clone keeps reusable_socket");
    }
}

mod connect_opts {
    use super::*;

    #[test]
    fn default_is_the_udp_form_without_reusable_socket() {
        let opts = ConnectOpts::default();

        assert!(
            !opts.reusable_socket,
            "false ↔ JS `dht.connect(key)` without options"
        );
    }

    #[test]
    fn default_has_no_relay() {
        assert_eq!(ConnectOpts::default().relay_through, None);
    }

    #[test]
    fn connect_opts_stay_copy_eq() {
        fn is_copy_eq<X: Copy + Eq>() {}
        is_copy_eq::<ConnectOpts>();
    }

    #[test]
    fn tcp_form_differs_from_default_and_copies_by_value() {
        let tcp = ConnectOpts {
            reusable_socket: true,
            relay_through: None,
        };

        let copy = tcp;

        assert_eq!(tcp, copy);
        assert_ne!(tcp, ConnectOpts::default());
    }
}

mod mutable_record {
    use super::*;

    #[test]
    fn records_are_equal_only_when_seq_and_value_both_match() {
        let record = MutableRecord {
            seq: 3,
            value: b"{\"port\":3000}".to_vec(),
        };

        assert_eq!(record, record.clone());
        assert_ne!(
            record,
            MutableRecord {
                seq: 4,
                ..record.clone()
            }
        );
        assert_ne!(
            record,
            MutableRecord {
                value: b"{\"port\":3001}".to_vec(),
                ..record.clone()
            }
        );
    }
}

mod connection {
    use super::*;

    #[test]
    fn debug_shows_the_remote_key_and_hides_the_stream() {
        let conn = DhtConnection::new([7; 32], Box::new(tokio::io::duplex(8).0));

        let text = format!("{conn:?}");

        assert!(
            text.starts_with("DhtConnection { remote_public_key: [7, 7,"),
            "{text}"
        );
        assert!(
            text.ends_with(", .. }"),
            "the stream is not printed, the field list is incomplete: {text}"
        );
    }

    #[tokio::test]
    async fn stream_field_reads_what_the_peer_wrote() {
        let (mut peer, ours) = tokio::io::duplex(64);
        let mut conn = DhtConnection::new([1; 32], Box::new(ours));

        peer.write_all(b"hi").await.unwrap();
        let mut buf = [0u8; 2];
        conn.stream.read_exact(&mut buf).await.unwrap();

        assert_eq!(&buf, b"hi");
    }
}

fn with_firewall(firewall: Firewall) -> ServerOpts {
    ServerOpts {
        firewall: Some(firewall),
        ..Default::default()
    }
}

fn ask(opts: &ServerOpts, key: &[u8; 32]) -> bool {
    let firewall = opts.firewall.as_ref().expect("firewall is set");
    firewall(key)
}

fn recording_firewall(seen: &Arc<Mutex<Vec<[u8; 32]>>>, answer: bool) -> Firewall {
    let seen = Arc::clone(seen);
    Arc::new(move |key: &[u8; 32]| {
        seen.lock().unwrap().push(*key);
        answer
    })
}
