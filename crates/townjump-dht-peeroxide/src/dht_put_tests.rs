// Tests of `mutable_put` on a node nobody answers for. JS: dht-rpc/lib/query.js `_endAfterCommit` ("Too few nodes responded").
use super::*;
use crate::factory::PeeroxideFactory;
use std::time::Duration;
use townjump_dht::DhtFactory;
use townjump_testkit::conformance::wait::within;
use townjump_testkit::key_pair;

const WAIT: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutable_put_without_responders_is_an_error() {
    // An empty bootstrap: no peers and no dead bootstrap entry to wait for.
    let factory = PeeroxideFactory {
        bootstrap: Some(vec![]),
        ..Default::default()
    };
    let node = within(WAIT, "create", factory.create(None))
        .await
        .expect("create");
    let pair = key_pair(1);
    let put = node.mutable_put(&pair, b"value", Some(1));
    let outcome = within(WAIT, "put", put).await;
    assert!(
        matches!(&outcome, Err(DhtError::Backend(m)) if m == "Too few nodes responded"),
        "got {outcome:?}"
    );
    let _ = node.destroy().await;
}
