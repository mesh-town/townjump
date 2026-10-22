// Tests of `cli::checks` (part 1): validateInput checks 1–4 — `--key`, the UDP warning, two keys.
// `--live` and `--port` are in `checks_ports_tests.rs`; checks 7–11, order and the CLI matrix in `checks_flow_tests.rs`; JS: holesail/src/lib/validateInput.js.
use super::*;
use townjump_jsval::JsValue;

mod key_without_value {
    use super::*;

    #[test]
    fn q4_bare_key_prints_red_error_and_exits_zero() {
        let outcome = check(args().flag("key", yes()));
        stops(outcome, KEY_EMPTY, 0);
    }

    #[test]
    fn q4_bare_key_next_to_live_still_exits_zero() {
        let outcome = check(args().flag("live", num(3000.0)).flag("key", yes()));
        stops(outcome, KEY_EMPTY, 0);
    }

    #[test]
    fn q4_bare_key_exits_before_the_udp_warning_is_printed() {
        let outcome = check(
            args()
                .flag("key", yes())
                .flag("udp", yes())
                .flag("host", text("localhost")),
        );
        stops(outcome, KEY_EMPTY, 0);
    }

    #[test]
    fn negated_key_is_falsy_and_passes() {
        passes(check(args().flag("key", JsValue::Bool(false))));
    }

    #[test]
    fn empty_string_key_is_falsy_and_passes() {
        passes(check(args().flag("key", text(""))));
    }
}

mod key_length {
    use super::*;

    #[test]
    fn three_char_key_stops_with_exit_2() {
        stops(
            check(args().flag("live", num(3000.0)).flag("key", text("abc"))),
            KEY_SHORT,
            2,
        );
    }

    #[test]
    fn thirty_one_char_key_stops_with_exit_2() {
        let key = text("abcdefghijklmnopqrstuvwxyz01234");
        stops(
            check(args().flag("live", num(3000.0)).flag("key", key)),
            KEY_SHORT,
            2,
        );
    }

    #[test]
    fn thirty_two_char_key_passes() {
        let key = text("abcdefghijklmnopqrstuvwxyz012345");
        passes(check(args().flag("live", num(3000.0)).flag("key", key)));
    }

    #[test]
    fn url_form_key_is_measured_as_a_whole_string() {
        stops(
            check(args().flag("key", text("hs://s000abc"))),
            KEY_SHORT,
            2,
        );
    }

    #[test]
    fn force_lifts_the_length_check() {
        passes(check(
            args()
                .flag("live", num(3000.0))
                .flag("key", text("abc"))
                .flag("force", yes()),
        ));
    }

    #[test]
    fn q6_force_false_string_is_truthy_and_still_lifts_the_check() {
        passes(check(
            args().flag("key", text("abc")).flag("force", text("false")),
        ));
    }

    #[test]
    fn negated_force_does_not_lift_the_check() {
        stops(
            check(
                args()
                    .flag("key", text("abc"))
                    .flag("force", JsValue::Bool(false)),
            ),
            KEY_SHORT,
            2,
        );
    }

    #[test]
    fn q5_numeric_key_has_no_length_and_bypasses_the_check() {
        passes(check(args().flag("key", num(123456.0))));
        passes(check(args().flag("key", num(1.2345678901234568e38))));
    }

    #[test]
    fn repeated_key_is_an_array_whose_item_count_is_its_length() {
        let two = JsValue::Array(vec![text("a"), text("b")]);
        stops(check(args().flag("key", two)), KEY_SHORT, 2);
        let many = JsValue::Array(vec![text("a"); 32]);
        passes(check(args().flag("key", many)));
    }

    #[test]
    fn length_is_counted_in_utf16_units_so_cyrillic_letters_are_short_not_long() {
        let key = text(&"ζ".repeat(31));
        stops(check(args().flag("key", key)), KEY_SHORT, 2);
    }

    #[test]
    fn an_emoji_counts_as_two_utf16_units() {
        passes(check(args().flag("key", text(&"😀".repeat(16)))));
        stops(
            check(args().flag("key", text(&"😀".repeat(15)))),
            KEY_SHORT,
            2,
        );
    }
}

mod udp_warning {
    use super::*;

    #[test]
    fn udp_with_localhost_warns_in_yellow_and_continues() {
        let outcome = check(
            args()
                .flag("live", num(3000.0))
                .flag("udp", yes())
                .flag("host", text("localhost")),
        );
        assert_eq!(
            outcome,
            CheckOutcome {
                stdout: yellow(UDP_WARN),
                exit: None
            }
        );
    }

    #[test]
    fn udp_with_all_interfaces_address_warns_too() {
        let outcome = check(
            args()
                .flag("live", num(3000.0))
                .flag("udp", yes())
                .flag("host", text("0.0.0.0")),
        );
        assert_eq!(
            outcome,
            CheckOutcome {
                stdout: yellow(UDP_WARN),
                exit: None
            }
        );
    }

    #[test]
    fn q6_udp_false_string_is_truthy_and_warns() {
        let outcome = check(
            args()
                .flag("udp", text("false"))
                .flag("host", text("localhost")),
        );
        assert_eq!(
            outcome,
            CheckOutcome {
                stdout: yellow(UDP_WARN),
                exit: None
            }
        );
    }

    #[test]
    fn udp_with_another_host_is_silent() {
        for host in ["127.0.0.1", "example.org", "LOCALHOST", "localhost "] {
            passes(check(args().flag("udp", yes()).flag("host", text(host))));
        }
    }

    #[test]
    fn localhost_without_udp_is_silent() {
        passes(check(
            args()
                .flag("live", num(3000.0))
                .flag("host", text("localhost")),
        ));
        passes(check(
            args()
                .flag("udp", JsValue::Bool(false))
                .flag("host", text("localhost")),
        ));
    }

    #[test]
    fn bare_host_flag_is_true_and_is_not_compared() {
        passes(check(args().flag("udp", yes()).flag("host", yes())));
    }

    #[test]
    fn warning_stays_in_stdout_before_a_later_error() {
        let outcome = check(
            args()
                .flag("udp", yes())
                .flag("host", text("localhost"))
                .flag("live", text("abc")),
        );
        let expected = format!("{}{}", yellow(UDP_WARN), red(BAD_PORT));
        assert_eq!(
            outcome,
            CheckOutcome {
                stdout: expected,
                exit: Some(2)
            }
        );
    }
}

mod two_connection_strings {
    use super::*;

    #[test]
    fn connect_flag_and_a_positional_stop_with_exit_2() {
        let outcome = check(args().flag("connect", text("key-one")).pos(text("key-two")));
        stops(outcome, TWO_STRINGS, 2);
    }

    #[test]
    fn numeric_connect_and_a_positional_stop_too() {
        stops(
            check(args().flag("connect", num(1234.0)).pos(text("x"))),
            TWO_STRINGS,
            2,
        );
    }

    #[test]
    fn connect_alone_passes() {
        passes(check(args().flag("connect", text("key-one"))));
    }

    #[test]
    fn positionals_alone_pass_whatever_their_number() {
        passes(check(args().pos(text("key-one")).pos(text("key-two"))));
    }

    #[test]
    fn falsy_first_positional_does_not_conflict() {
        passes(check(args().flag("connect", text("key-one")).pos(num(0.0))));
        passes(check(args().flag("connect", text("key-one")).pos(text(""))));
    }
}

pub(super) const KEY_EMPTY: &str = "Error: Key can not be empty";
pub(super) const UDP_WARN: &str =
    "Warning: localhost or 0.0.0.0 may not work properly within netcat with UDP.";
pub(super) const KEY_SHORT: &str = "Error: A key should have a minimum length of 32 chars for security purposes. If you still wish to proceed use --force";
pub(super) const TWO_STRINGS: &str =
    "Error: Are you trying to use two connection strings at once? Get some holesail --help";
pub(super) const BAD_PORT: &str =
    "Error: Given port is not a valid number. Run holesail --help to see examples";

/// Parsed argv as `minimist` does it: `_` first, then the flags in order of addition.
pub(super) struct Args {
    flags: Vec<(String, JsValue)>,
    positional: Vec<JsValue>,
}

pub(super) fn args() -> Args {
    Args {
        flags: Vec::new(),
        positional: Vec::new(),
    }
}

impl Args {
    pub(super) fn flag(mut self, name: &str, value: JsValue) -> Self {
        self.flags.push((name.to_owned(), value));
        self
    }

    pub(super) fn pos(mut self, value: JsValue) -> Self {
        self.positional.push(value);
        self
    }

    pub(super) fn build(self) -> JsValue {
        let mut entries = vec![("_".to_owned(), JsValue::Array(self.positional))];
        entries.extend(self.flags);
        JsValue::Object(entries)
    }
}

pub(super) fn yes() -> JsValue {
    JsValue::Bool(true)
}

pub(super) fn num(n: f64) -> JsValue {
    JsValue::Number(n)
}

pub(super) fn text(s: &str) -> JsValue {
    JsValue::String(s.to_owned())
}

/// `barely-colours`: red and yellow, the line ends with a line break (`console.log`).
pub(super) fn red(message: &str) -> String {
    format!("\u{1b}[31m{message}\u{1b}[0m\n")
}

pub(super) fn yellow(message: &str) -> String {
    format!("\u{1b}[33m{message}\u{1b}[0m\n")
}

/// A check in which every path exists.
pub(super) fn check(args: Args) -> CheckOutcome {
    validate_input(&args.build(), &|_| true)
}

pub(super) fn stops(outcome: CheckOutcome, message: &str, code: i32) {
    assert_eq!(
        outcome,
        CheckOutcome {
            stdout: red(message),
            exit: Some(code)
        }
    );
}

pub(super) fn passes(outcome: CheckOutcome) {
    assert_eq!(
        outcome,
        CheckOutcome {
            stdout: String::new(),
            exit: None
        }
    );
}
