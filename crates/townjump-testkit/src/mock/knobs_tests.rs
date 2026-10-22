// Tests for the mock gates and injected errors: `hold_*`, `failing_*`, `release_*`, `put_calls`.
// "Did not complete" is a short `timeout` on the same future (without cancellation), no real pauses.
use crate::conformance::key_pair;
use crate::mock::test_util::node;
use crate::mock::{MockConfig, MockFactory};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::timeout;
use townjump_dht::{Dht, DhtError, DhtServer, ServerOpts};

/// How long we wait to make sure the call is stuck.
const HELD: Duration = Duration::from_millis(50);
/// Limit for waiting for completion; not spent on the success path.
const LIMIT: Duration = Duration::from_secs(5);

async fn is_held<F: Future>(call: &mut Pin<&mut F>) -> bool {
    timeout(HELD, call).await.is_err()
}

async fn finished<F: Future>(call: &mut Pin<&mut F>) -> F::Output {
    let done = timeout(LIMIT, call).await;
    done.expect("call did not complete within 5 s")
}

fn factory(config: MockConfig) -> MockFactory {
    MockFactory::with_config(config)
}

fn is_backend(result: Result<impl Sized, DhtError>, text: &str) -> bool {
    matches!(result, Err(DhtError::Backend(ref t)) if t == text)
}

mod hold_listen {
    use super::*;

    #[tokio::test]
    async fn waits_for_release_then_does_the_real_work() {
        let net = factory(MockConfig::default().hold_listen(true));
        let dht = node(&net, 1).await;
        let (server, _incoming) = dht.create_server(ServerOpts::default()).unwrap();
        let listen = server.listen(key_pair(1));
        tokio::pin!(listen);
        assert!(is_held(&mut listen).await, "listen passed with no release");
        assert!(
            net.trace().named("server.listen").is_empty(),
            "work before the gate"
        );
        net.release_listen();
        finished(&mut listen).await.unwrap();
        assert_eq!(net.trace().named("server.listen").len(), 1);
    }

    #[tokio::test]
    async fn release_before_the_call_lets_every_call_pass() {
        let net = factory(MockConfig::default().hold_listen(true));
        net.release_listen();
        let dht = node(&net, 1).await;
        for tag in [1, 2] {
            let (server, _incoming) = dht.create_server(ServerOpts::default()).unwrap();
            let listen = server.listen(key_pair(tag));
            tokio::pin!(listen);
            finished(&mut listen).await.unwrap();
        }
    }

    #[tokio::test]
    async fn does_not_hold_put() {
        let net = factory(MockConfig::default().hold_listen(true));
        let dht = node(&net, 1).await;
        let kp = key_pair(1);
        let put = dht.mutable_put(&kp, b"v", None);
        tokio::pin!(put);
        assert_eq!(finished(&mut put).await.unwrap(), 0);
    }
}

mod hold_put {
    use super::*;

    #[tokio::test]
    async fn waits_for_release_then_does_the_real_work() {
        let net = factory(MockConfig::default().hold_put(true));
        let dht = node(&net, 1).await;
        let kp = key_pair(1);
        let put = dht.mutable_put(&kp, b"v", Some(4));
        tokio::pin!(put);
        assert!(is_held(&mut put).await, "put passed without a release");
        assert!(
            net.trace().named("mutablePut").is_empty(),
            "work before the gate"
        );
        assert_eq!(net.put_calls(), 1, "the stuck call is already counted");
        net.release_put();
        assert_eq!(finished(&mut put).await.unwrap(), 4);
        assert_eq!(net.trace().named("mutablePut").len(), 1);
    }

    #[tokio::test]
    async fn one_release_frees_all_waiting_calls() {
        let net = factory(MockConfig::default().hold_put(true));
        let dht = node(&net, 1).await;
        let kp = key_pair(1);
        let (first, second) = (
            dht.mutable_put(&kp, b"a", None),
            dht.mutable_put(&kp, b"b", None),
        );
        tokio::pin!(first, second);
        assert!(is_held(&mut first).await && is_held(&mut second).await);
        net.release_put();
        finished(&mut first).await.unwrap();
        finished(&mut second).await.unwrap();
    }

    #[tokio::test]
    async fn does_not_hold_listen() {
        let net = factory(MockConfig::default().hold_put(true));
        let dht = node(&net, 1).await;
        let (server, _incoming) = dht.create_server(ServerOpts::default()).unwrap();
        let listen = server.listen(key_pair(1));
        tokio::pin!(listen);
        finished(&mut listen).await.unwrap();
    }
}

mod failing_listen {
    use super::*;

    #[tokio::test]
    async fn answers_backend_error_before_the_real_work() {
        let net = factory(MockConfig::default().failing_listen("listen failed"));
        let dht = node(&net, 1).await;
        let (server, _incoming) = dht.create_server(ServerOpts::default()).unwrap();
        let result = server.listen(key_pair(1)).await;
        assert!(is_backend(result, "listen failed"));
        assert!(net.trace().named("server.listen").is_empty());
    }
}

mod failing_put_from {
    use super::*;

    #[tokio::test]
    async fn second_and_later_puts_fail_first_is_fine() {
        let net = factory(MockConfig::default().failing_put_from(2));
        let dht = node(&net, 1).await;
        let kp = key_pair(1);
        assert_eq!(dht.mutable_put(&kp, b"a", Some(1)).await.unwrap(), 1);
        assert!(is_backend(
            dht.mutable_put(&kp, b"b", Some(2)).await,
            "put failed"
        ));
        assert!(is_backend(
            dht.mutable_put(&kp, b"c", Some(3)).await,
            "put failed"
        ));
        let stored = dht.mutable_get(kp.public_key, None).await.unwrap();
        assert_eq!(
            stored.map(|record| record.seq),
            Some(1),
            "failed puts are not stored"
        );
    }

    #[tokio::test]
    async fn from_the_first_call_fails_everything() {
        let net = factory(MockConfig::default().failing_put_from(1));
        let dht = node(&net, 1).await;
        let result = dht.mutable_put(&key_pair(1), b"a", None).await;
        assert!(is_backend(result, "put failed"));
        assert!(
            net.trace().named("mutablePut").is_empty(),
            "error before the real work"
        );
    }
}

mod put_calls {
    use super::*;

    #[tokio::test]
    async fn counts_ok_and_failed_calls_of_all_nodes() {
        let net = factory(MockConfig::default().failing_put_from(3));
        assert_eq!(net.put_calls(), 0);
        for tag in [1, 2, 3, 4] {
            let dht = node(&net, tag).await;
            let _ignored = dht.mutable_put(&key_pair(tag), b"v", None).await;
        }
        assert_eq!(net.put_calls(), 4);
    }

    #[tokio::test]
    async fn default_config_counts_and_release_is_harmless() {
        let net = MockFactory::new();
        net.release_listen();
        net.release_put();
        let dht = node(&net, 1).await;
        dht.mutable_put(&key_pair(1), b"v", None).await.unwrap();
        assert_eq!(net.put_calls(), 1);
    }
}

mod sharing {
    use super::*;

    #[tokio::test]
    async fn clones_share_gates_and_counter() {
        let net = factory(MockConfig::default().hold_put(true));
        let clone = net.clone();
        let dht = node(&clone, 1).await;
        let kp = key_pair(1);
        let put = dht.mutable_put(&kp, b"v", None);
        tokio::pin!(put);
        assert!(is_held(&mut put).await);
        assert_eq!((net.put_calls(), clone.put_calls()), (1, 1));
        net.release_put();
        finished(&mut put).await.unwrap();
    }

    #[tokio::test]
    async fn separate_factories_do_not_share() {
        let (first, second) = (
            factory(MockConfig::default().hold_put(true)),
            factory(MockConfig::default().hold_put(true)),
        );
        let dht = node(&second, 1).await;
        let kp = key_pair(1);
        let put = dht.mutable_put(&kp, b"v", None);
        tokio::pin!(put);
        first.release_put();
        assert!(is_held(&mut put).await, "a foreign release opened the gate");
        assert_eq!((first.put_calls(), second.put_calls()), (0, 1));
    }
}
