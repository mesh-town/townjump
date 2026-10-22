// Shared suite on `MockFactory`: one test per scenario. A multi-threaded runtime, like the adapter's live network.
use super::*;
use crate::mock::MockFactory;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_1mb_on_mock() {
    stream_1mb(&MockFactory::new()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn half_close_on_mock() {
    half_close(&MockFactory::new()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_key_five_connects_on_mock() {
    one_key_five_connects(&MockFactory::new()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn firewall_foreign_key_on_mock() {
    firewall_foreign_key(&MockFactory::new()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutable_record_seq_on_mock() {
    mutable_record_seq(&MockFactory::new()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspend_resume_on_mock() {
    suspend_resume(&MockFactory::new()).await;
}
