// CLI tests over `testvectors/holesail-cli/cli-matrix.json`: `run` with `MockFactory`, stdout, stderr, code, trace.
// One test per matrix case, the test name is the case id; the case check and output normalization are in `cli_matrix/support.rs`.
use support::check;
use townjump_testkit::vectors;

#[path = "cli_matrix/support.rs"]
mod support;

/// Tests per case: `name: "id"`; after `=>` — the error text for cases with code 1 (the Node stack is not compared).
macro_rules! matrix {
    (@needle) => { None };
    (@needle $text:literal) => { Some($text) };
    ($($name:ident: $id:literal $(=> $needle:literal)?),* $(,)?) => {
        pub const IDS: &[&str] = &[$($id),*];
        $(
            #[test]
            fn $name() {
                check($id, matrix!(@needle $($needle)?));
            }
        )*
    };
}

// Help, version and unknown flags: code 0, nothing goes to the DHT (`-v` gives the help).
mod help_and_version {
    use super::*;

    matrix! {
        help_noargs: "help-noargs",
        help_flag: "help-flag",
        help_short: "help-short",
        help_live: "help-live",
        help_connect: "help-connect",
        help_filemanager: "help-filemanager",
        help_unknown: "help-unknown",
        help_short_arg: "help-short-arg",
        version: "version",
        version_short: "version-short",
        version_before_live: "version-before-live",
        unknown_flag: "unknown-flag",
        unknown_positional_only_flag_arg: "unknown-positional-only-flag-arg",
    }
}

// Input checks before startup: code 2 and the text on stderr (help is not printed after an error).
mod input_checks {
    use super::*;

    matrix! {
        help_after_invalid: "help-after-invalid",
        live_bare: "live-bare",
        live_abc: "live-abc",
        live_zero: "live-zero",
        live_neg: "live-neg",
        live_twice: "live-twice",
        port_abc: "port-abc",
        port_empty: "port-empty",
        port_bare: "port-bare",
        key_bare: "key-bare",
        key_bare_live: "key-bare-live",
        key_short: "key-short",
        key_31: "key-31",
        role_bad: "role-bad",
        connect_two: "connect-two",
        fm_udp: "fm-udp",
        fm_live: "fm-live",
        fm_connect: "fm-connect",
        fm_connect_pos: "fm-connect-pos",
        fm_nopath: "fm-nopath",
    }
}

// `--live`: the server banner and the DHT trace; runs until stopped.
mod live_server {
    use super::*;

    matrix! {
        live_float: "live-float",
        live_hex: "live-hex",
        live_eq: "live-eq",
        key_short_force: "key-short-force",
        udp_warn_localhost: "udp-warn-localhost",
        udp_warn_0000: "udp-warn-0000",
        live_private: "live-private",
        live_public: "live-public",
        live_public_eq: "live-public-eq",
        live_udp: "live-udp",
        live_host: "live-host",
        live_key40: "live-key40",
        live_key32: "live-key32",
        live_key40_public: "live-key40-public",
        live_key_url_private: "live-key-url-private",
        live_key_url_private_public: "live-key-url-private-public",
        live_key_url_public: "live-key-url-public",
        live_key_url_shortkey_force: "live-key-url-shortkey-force",
        live_key_hex_b: "live-key-hex-b",
        live_port_ignored: "live-port-ignored",
        live_big_port: "live-big-port",
    }
}

// `--live` with `--log`: log lines by level; a secret in the log is a fingerprint.
mod live_logs {
    use super::*;

    matrix! {
        live_log: "live-log",
        live_log0: "live-log0",
        live_log9: "live-log9",
        live_log_abc: "live-log-abc",
        live_log_eq2: "live-log-eq2",
        live_fake_conn: "live-fake-conn",
    }
}

// `--connect` and a positional key: mode by key, DHT record, errors with code 1.
mod client {
    use super::*;

    matrix! {
        conn_public_z32: "conn-public-z32",
        conn_pos_public_z32: "conn-pos-public-z32",
        conn_pos_two: "conn-pos-two",
        conn_z32_6s: "conn-z32-6s",
        conn_url_0000: "conn-url-0000",
        conn_url_s000: "conn-url-s000",
        conn_url_s000_public_flag: "conn-url-s000-public-flag",
        conn_hex_noprefix: "conn-hex-noprefix" => "Invalid character in base32 input: \"2\" at position 3",
        conn_hex_noprefix_public_flag: "conn-hex-noprefix-public-flag",
        conn_z32_public_flag: "conn-z32-public-flag",
        conn_invalid_chars: "conn-invalid-chars" => "Invalid character in base32 input: \"!\" at position 3",
        conn_default_port: "conn-default-port",
        conn_record_host_port: "conn-record-host-port" => "listen EADDRNOTAVAIL: address not available 10.1.2.3:4000", // JS stderr in cli-matrix.json; Rust builds the text from `io::ErrorKind` after Node's pattern
        conn_record_udp: "conn-record-udp",
        conn_record_override: "conn-record-override",
        conn_record_badjson: "conn-record-badjson" => "Unexpected token 'o', \"not json\" is not valid JSON",
        conn_udp: "conn-udp",
        conn_log: "conn-log",
        conn_port_zero: "conn-port-zero",
        conn_port_inuse: "conn-port-inuse" => "listen EADDRINUSE: address already in use 127.0.0.1:18920", // JS stderr in cli-matrix.json; Rust builds the text from `io::ErrorKind` after Node's pattern
        conn_eq_syntax: "conn-eq-syntax",
    }
}

// `--lookup`: the result and errors, the code is always 0.
mod lookup {
    use super::*;

    matrix! {
        lookup_none: "lookup-none",
        lookup_bare: "lookup-bare",
        lookup_invalid: "lookup-invalid",
        lookup_found_public: "lookup-found-public",
        lookup_found_udp: "lookup-found-udp",
        lookup_url_private: "lookup-url-private",
        lookup_url_public: "lookup-url-public",
        lookup_badjson: "lookup-badjson",
        lookup_empty_obj: "lookup-empty-obj",
    }
}

// `--filemanager` with startup: banner, port, stop on a signal (130), startup errors (code 1).
// `fm-default` listens on 5409, as JS does (`support.rs` checks that the port is free); `fm-port-inuse` holds 18932.
mod filemanager {
    use super::*;

    matrix! {
        fm_default: "fm-default",
        fm_opts: "fm-opts",
        fm_key: "fm-key",
        fm_port_inuse: "fm-port-inuse" => "listen EADDRINUSE: address already in use 127.0.0.1:18932", // JS stderr in cli-matrix.json
        fm_bare: "fm-bare" => "INCORRECT OR NO PATH SPECIFIED", // the Node stack in JS stderr is not copied
    }
}

#[test]
fn every_json_case_has_a_test() {
    let matrix = vectors::load_value("holesail-cli/cli-matrix.json");
    let cases = matrix.as_array().expect("the matrix is an array");
    let mut in_json: Vec<&str> = cases
        .iter()
        .map(|c| c["id"].as_str().expect("id"))
        .collect();
    let mut listed: Vec<&str> = [
        help_and_version::IDS,
        input_checks::IDS,
        live_server::IDS,
        live_logs::IDS,
        client::IDS,
        lookup::IDS,
        filemanager::IDS,
    ]
    .concat();
    in_json.sort_unstable();
    listed.sort_unstable();
    assert_eq!(listed, in_json, "macro ids differ from JSON ids");
}
