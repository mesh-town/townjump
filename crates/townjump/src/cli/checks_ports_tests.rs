// Tests of `cli::checks` (part 3): validateInput checks 5–6 — `--live` and `--port` must be numbers (no bounds, as in JS).
// Helpers are in `checks_tests.rs`; JS: holesail/src/lib/validateInput.js.
use super::tests::{BAD_PORT, args, check, num, passes, red, stops, text, yes};
use super::*;
use townjump_jsval::JsValue;

mod live_port {
    use super::*;

    #[test]
    fn truthy_live_that_is_not_a_number_stops_with_exit_2() {
        let repeated = JsValue::Array(vec![num(1.0), num(2.0)]);
        for (label, live) in [
            ("bare", yes()),
            ("word", text("abc")),
            ("repeated", repeated),
            ("spaced", text(" 5")),
        ] {
            let outcome = check(args().flag("live", live));
            assert_eq!(
                outcome,
                CheckOutcome {
                    stdout: red(BAD_PORT),
                    exit: Some(2)
                },
                "live: {label}"
            );
        }
    }

    #[test]
    fn any_number_passes_without_range_checks() {
        for port in [3000.0, 3000.5, 16.0, 99999.0, f64::INFINITY] {
            passes(check(args().flag("live", num(port))));
        }
    }

    #[test]
    fn q13_zero_live_is_falsy_and_passes() {
        passes(check(args().flag("live", num(0.0))));
    }

    #[test]
    fn empty_string_live_is_falsy_and_passes() {
        passes(check(args().flag("live", text(""))));
    }

    #[test]
    fn help_flag_does_not_shield_an_invalid_live() {
        stops(
            check(args().flag("live", text("abc")).flag("help", yes())),
            BAD_PORT,
            2,
        );
    }
}

mod port {
    use super::*;

    #[test]
    fn word_port_stops_with_exit_2() {
        let outcome = check(
            args()
                .flag("port", text("abc"))
                .flag("connect", text("key")),
        );
        stops(outcome, BAD_PORT, 2);
    }

    #[test]
    fn bare_port_is_true_and_stops() {
        stops(
            check(args().flag("port", yes()).flag("connect", text("key"))),
            BAD_PORT,
            2,
        );
    }

    #[test]
    fn repeated_port_is_an_array_and_stops() {
        let repeated = JsValue::Array(vec![num(8989.0), num(9000.0)]);
        stops(check(args().flag("port", repeated)), BAD_PORT, 2);
    }

    #[test]
    fn empty_string_port_stops_through_the_second_branch() {
        stops(
            check(args().flag("port", text("")).flag("connect", text("key"))),
            BAD_PORT,
            2,
        );
    }

    #[test]
    fn q13_zero_port_passes() {
        passes(check(
            args().flag("port", num(0.0)).flag("connect", text("key")),
        ));
    }

    #[test]
    fn numbers_pass_without_range_checks() {
        for port in [18901.0, 3000.5, 99999.0] {
            passes(check(args().flag("port", num(port))));
        }
    }

    #[test]
    fn negated_port_is_false_and_passes() {
        passes(check(args().flag("port", JsValue::Bool(false))));
    }
}
