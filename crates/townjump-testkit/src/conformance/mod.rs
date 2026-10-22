// JS: hyperdht/lib (behaviour the scenarios check); live network - interop/js/testnet.mjs via JsTestnet.
// Shared suite: each scenario is `async fn <name><F: DhtFactory>(factory: &F)` and panics with the text of the violated condition.
// For `MockFactory` they run from mock_run_tests, for the fork adapter from its JS-testnet tests; broken_tests prove the scenarios turn red.
// Files: keys - key pairs; wait - waits with a limit; setup - nodes and `Listening`; data - byte pattern and exchange;
// stream (`stream_1mb`, `half_close`), one_key, firewall, record, suspend - one scenario each.
pub mod data;
mod firewall;
pub mod keys;
mod one_key;
mod record;
pub mod setup;
mod stream;
mod suspend;
pub mod wait;

#[cfg(test)]
mod broken_tests;
#[cfg(test)]
mod mock_run_tests;

pub use firewall::firewall_foreign_key;
pub use keys::{key_pair, key_pair_from_seed};
pub use one_key::one_key_five_connects;
pub use record::mutable_record_seq;
pub use stream::{half_close, stream_1mb};
pub use suspend::suspend_resume;
