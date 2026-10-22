// Tests of `init_tracing`. We do not change the environment: in edition 2024 `set_var` is unsafe, while the workspace has
// `unsafe_code = "forbid"`; so the filter is passed to `init_tracing_with` explicitly.
use super::*;

#[test]
fn without_filter_no_subscriber() {
    assert!(!init_tracing_with(None));
}

#[test]
fn unset_rust_log_installs_nothing() {
    if std::env::var_os("RUST_LOG").is_none() {
        assert!(!init_tracing());
    }
}

#[test]
fn with_filter_installs_and_repeat_does_not_panic() {
    // The only test that installs a global subscriber in this process.
    assert!(init_tracing_with(Some(EnvFilter::new("debug"))));
    assert!(!init_tracing_with(Some(EnvFilter::new("debug"))));
}
