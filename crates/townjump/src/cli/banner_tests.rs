// Tests of `cli::banner`: stdout bytes of the banner for server and client cases of the CLI matrix (frame, colors, QR) and the frame structure.
// The banner prints the connection string in full, like JS (secret masking is only in logs, not in output); JS: holesail/src/lib/stdout.js.
use super::*;
use crate::cli::qr::qr_small;
use townjump_jsval::JsValue;
use townjump_testkit::vectors::load_value;

const PRIVATE_SERVER: &str =
    "hs://s000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const PUBLIC_SERVER: &str = "hs://0000yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";
const PUBLIC_CLIENT: &str = "hs://0000ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
const PRIVATE_CLIENT: &str = "hs://s000ybndrsg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqxot1";
const SHORT_KEY: &str = "hs://s000abcdefghijklmnopqrstuvwxyz0123456789ABCD";
const HEX_KEY: &str = "hs://s000a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

mod server_matches_js {
    use super::*;

    #[test]
    fn private_tcp_server() {
        assert_js(
            "live-private",
            server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER),
        );
    }

    #[test]
    fn public_tcp_server_has_the_wider_box() {
        assert_js(
            "live-public",
            server("tcp", false, "127.0.0.1", 3000.0, PUBLIC_SERVER),
        );
    }

    #[test]
    fn udp_server_says_udp_in_the_title() {
        assert_js(
            "live-udp",
            server("udp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER),
        );
    }

    #[test]
    fn custom_host_is_printed_as_given() {
        assert_js(
            "live-host",
            server("tcp", true, "0.0.0.0", 3000.0, PRIVATE_SERVER),
        );
    }

    #[test]
    fn q13_fractional_port_is_printed_as_is() {
        assert_js(
            "live-float",
            server("tcp", true, "127.0.0.1", 3000.5, PRIVATE_SERVER),
        );
    }

    #[test]
    fn q5_hex_port_is_printed_in_decimal() {
        assert_js(
            "live-hex",
            server("tcp", true, "127.0.0.1", 16.0, PRIVATE_SERVER),
        );
    }

    #[test]
    fn q13_port_above_65535_is_printed_without_checks() {
        assert_js(
            "live-big-port",
            server("tcp", true, "127.0.0.1", 99999.0, PRIVATE_SERVER),
        );
    }

    #[test]
    fn custom_short_key_gives_a_shorter_url_and_a_smaller_qr() {
        assert_js(
            "live-key40",
            server("tcp", true, "127.0.0.1", 3000.0, SHORT_KEY),
        );
    }

    #[test]
    fn custom_hex_key_url() {
        assert_js(
            "live-key-hex-b",
            server("tcp", true, "127.0.0.1", 3000.0, HEX_KEY),
        );
    }
}

mod client_matches_js {
    use super::*;

    #[test]
    fn public_tcp_client_shows_the_access_url_and_no_qr() {
        assert_js(
            "conn-public-z32",
            client("tcp", false, "127.0.0.1", 18901.0, PUBLIC_CLIENT),
        );
    }

    #[test]
    fn private_client_shows_the_note_line() {
        assert_js(
            "conn-z32-6s",
            client("tcp", true, "127.0.0.1", 18904.0, PRIVATE_CLIENT),
        );
    }

    #[test]
    fn udp_client_says_udp_in_the_title() {
        assert_js(
            "conn-udp",
            client("udp", false, "127.0.0.1", 18914.0, PUBLIC_CLIENT),
        );
    }

    #[test]
    fn default_port_8989() {
        assert_js(
            "conn-default-port",
            client("tcp", false, "127.0.0.1", 8989.0, PUBLIC_CLIENT),
        );
    }

    #[test]
    fn host_and_port_from_the_dht_record() {
        assert_js(
            "conn-record-udp",
            client("udp", false, "0.0.0.0", 4100.0, PUBLIC_CLIENT),
        );
    }

    #[test]
    fn port_zero_is_printed_as_zero() {
        assert_js(
            "conn-port-zero",
            client("tcp", false, "127.0.0.1", 0.0, PUBLIC_CLIENT),
        );
    }
}

mod structure {
    use super::*;

    #[test]
    fn private_box_is_121_columns_wide_and_12_lines_tall() {
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER));
        assert_eq!(box_widths(&out), vec![121; 12]);
    }

    #[test]
    fn public_box_is_190_columns_wide_and_12_lines_tall() {
        let out = banner(&client("tcp", false, "127.0.0.1", 8989.0, PUBLIC_CLIENT));
        assert_eq!(box_widths(&out), vec![190; 12]);
    }

    #[test]
    fn a_long_url_widens_the_box_to_the_key_line() {
        let url = format!("hs://s000{}", "a".repeat(200));
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, &url));
        assert_eq!(box_widths(&out), vec![229; 12]);
        let lines: Vec<String> = out.lines().map(visible).collect();
        assert_eq!(lines[0], format!("┌{}┐", "─".repeat(227)));
        assert_eq!(lines[11], format!("└{}┘", "─".repeat(227)));
        assert_eq!(
            lines[6],
            format!("|Connect with key: {url}│"),
            "the longest line without padding"
        );
    }

    #[test]
    fn border_is_asymmetric_ascii_pipe_on_the_left_box_bar_on_the_right() {
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER));
        for line in out.lines().skip(1).take(10).map(visible) {
            assert!(
                line.starts_with('|') && line.ends_with('│'),
                "line frame: {line:?}"
            );
        }
    }

    #[test]
    fn heading_is_cyan_underline_bold_and_the_boat_is_outside_the_wrapper() {
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER));
        let heading =
            "\u{1b}[36m\u{1b}[4m\u{1b}[1mHolesail TCP Server Started\u{1b}[0m\u{1b}[0m\u{1b}[0m ⛵️";
        assert!(
            out.contains(heading),
            "header with ANSI and the boat outside the wrapper"
        );
    }

    #[test]
    fn server_output_ends_with_the_qr_of_the_url() {
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER));
        assert!(
            out.ends_with(&qr_small(PRIVATE_SERVER)),
            "after the frame — the QR of the connection string"
        );
    }

    #[test]
    fn client_output_ends_with_the_bottom_border_and_has_no_qr() {
        let out = banner(&client("tcp", true, "127.0.0.1", 18904.0, PRIVATE_CLIENT));
        assert!(
            out.ends_with("┘\n"),
            "the client banner ends with the frame: {:?}",
            out.lines().last()
        );
    }

    #[test]
    fn q9_url_is_printed_in_full_and_the_other_info_fields_are_not_printed_at_all() {
        let out = banner(&server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER));
        assert!(
            out.contains(PRIVATE_SERVER),
            "the connection string is printed in full, as in JS"
        );
        assert!(
            !out.contains("****"),
            "secret masking is only in logs, not in output"
        );
        for marker in [KEY_FIELD, SEED_FIELD, PUBLIC_KEY_FIELD] {
            assert!(
                !out.contains(marker),
                "the banner takes only `url`, not {marker}"
            );
        }
    }
}

mod values_are_printed_like_js_template_strings {
    use super::*;

    #[test]
    fn numbers_use_javascript_number_formatting() {
        for (port, printed) in [
            (1e21, "1e+21"),
            (f64::INFINITY, "Infinity"),
            (0.1, "0.1"),
            (1e-7, "1e-7"),
        ] {
            let server_line = format!("Holesail is now listening on 127.0.0.1:{printed}");
            let client_line = format!("Access application on http://127.0.0.1:{printed}/");
            let out = banner(&server("tcp", true, "127.0.0.1", port, PRIVATE_SERVER));
            assert!(
                visible(&out).contains(&server_line),
                "port {port}: expected `{server_line}`"
            );
            let out = banner(&client("tcp", true, "127.0.0.1", port, PRIVATE_CLIENT));
            assert!(
                visible(&out).contains(&client_line),
                "port {port}: expected `{client_line}`"
            );
        }
    }

    #[test]
    fn bare_host_flag_is_true_and_prints_as_true() {
        let mut info = server("tcp", true, "127.0.0.1", 3000.0, PRIVATE_SERVER);
        info.host = JsValue::Bool(true);
        assert!(visible(&banner(&info)).contains("Holesail is now listening on true:3000"));
    }
}

const KEY_FIELD: &str = "MARKER-KEY-FIELD";
const SEED_FIELD: &str = "MARKER-SEED-FIELD";
const PUBLIC_KEY_FIELD: &str = "MARKER-PUBLIC-KEY-FIELD";

fn info(
    kind: &'static str,
    protocol: &'static str,
    secure: bool,
    host: &str,
    port: f64,
    url: &str,
) -> HolesailInfo {
    HolesailInfo {
        kind,
        state: Some("listening"),
        secure,
        port: JsValue::Number(port),
        host: JsValue::String(host.to_owned()),
        protocol,
        seed: Some(SEED_FIELD.to_owned()),
        key: KEY_FIELD.to_owned(),
        url: url.to_owned(),
        public_key: PUBLIC_KEY_FIELD.to_owned(),
    }
}

fn server(protocol: &'static str, secure: bool, host: &str, port: f64, url: &str) -> HolesailInfo {
    info("server", protocol, secure, host, port, url)
}

fn client(protocol: &'static str, secure: bool, host: &str, port: f64, url: &str) -> HolesailInfo {
    info("client", protocol, secure, host, port, url)
}

/// Strips the ANSI sequences `\e[…m`.
fn visible(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            chars.by_ref().find(|&x| x == 'm');
        } else {
            out.push(c);
        }
    }
    out
}

/// Width in characters of the first 12 lines (top, 10 inner, bottom) without ANSI.
fn box_widths(out: &str) -> Vec<usize> {
    out.lines()
        .take(12)
        .map(|l| visible(l).chars().count())
        .collect()
}

fn matrix_stdout(id: &str) -> String {
    let matrix = load_value("holesail-cli/cli-matrix.json");
    let case = matrix
        .as_array()
        .and_then(|all| all.iter().find(|c| c["id"] == id));
    case.unwrap_or_else(|| panic!("no case {id} in the matrix"))["stdout"]
        .as_str()
        .expect("stdout")
        .to_owned()
}

/// The banner equals the matrix case stdout byte for byte (in these cases there is nothing but the banner in stdout).
fn assert_js(id: &str, info: HolesailInfo) {
    let (got, want) = (banner(&info), matrix_stdout(id));
    if got != want {
        panic!(
            "case {id}: the first difference — {}",
            first_diff(&got, &want)
        );
    }
}

fn first_diff(got: &str, want: &str) -> String {
    let (g, w): (Vec<&str>, Vec<&str>) = (got.split('\n').collect(), want.split('\n').collect());
    let i = (0..g.len().max(w.len()))
        .find(|&i| g.get(i) != w.get(i))
        .unwrap_or(0);
    format!("line {i}\n  got {:?}\n  expected {:?}", g.get(i), w.get(i))
}
