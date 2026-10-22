// Tests of `cli::checks` (part 2): validateInput checks 7–11 (filemanager, role), check order and the CLI matrix.
// Helpers and checks 1–6 are in `checks_tests.rs`; JS: holesail/src/lib/validateInput.js.
use super::tests::{
    Args, BAD_PORT, KEY_EMPTY, KEY_SHORT, TWO_STRINGS, UDP_WARN, args, check, num, passes, red,
    stops, text, yellow, yes,
};
use super::*;
use crate::cli::argv::parse_argv;
use std::cell::RefCell;
use townjump_jsval::JsValue;
use townjump_testkit::vectors::load_value;

const FM_UDP: &str = "Error: You can't run filemanager in UDP mode.";
const FM_LIVE: &str = "Error: You can't start holesail server and filemanager at the same time. If you are trying to use filemanager on a specific local port use --port instead or see holesail --help";
const FM_CONNECT: &str = "Error: You tried to create a connection and start filemanager both at once. Start them separately and check your command for mistakes. See holesail --help";
const NO_PATH: &str = "Error: Given path does not exist";
const BAD_ROLE: &str = "Error: Incorrect role set. Role can be either \"admin\" or \"user\" ";

mod filemanager {
    use super::*;

    #[test]
    fn with_udp_it_stops_with_exit_2() {
        stops(
            check(args().flag("filemanager", text(".")).flag("udp", yes())),
            FM_UDP,
            2,
        );
    }

    #[test]
    fn with_live_it_stops_with_exit_2() {
        stops(
            check(
                args()
                    .flag("filemanager", text("."))
                    .flag("live", num(3000.0)),
            ),
            FM_LIVE,
            2,
        );
    }

    #[test]
    fn with_connect_flag_it_stops_with_exit_2() {
        stops(
            check(
                args()
                    .flag("filemanager", text("."))
                    .flag("connect", text("key")),
            ),
            FM_CONNECT,
            2,
        );
    }

    #[test]
    fn with_a_positional_key_it_stops_with_exit_2() {
        stops(
            check(args().flag("filemanager", text(".")).pos(text("key"))),
            FM_CONNECT,
            2,
        );
    }

    #[test]
    fn missing_path_stops_with_exit_2() {
        let outcome = validate_input(
            &args()
                .flag("filemanager", text("/nonexistent/path"))
                .build(),
            &|_| false,
        );
        assert_eq!(outcome.exit, Some(2));
        assert_eq!(outcome.stdout, red(NO_PATH));
    }

    #[test]
    fn existing_path_passes_and_the_path_reaches_the_callback_unchanged() {
        let seen = RefCell::new(Vec::new());
        let argv = args().flag("filemanager", text("/tmp/a b")).build();
        let outcome = validate_input(&argv, &|p| {
            seen.borrow_mut().push(p.to_owned());
            true
        });
        passes(outcome);
        assert_eq!(
            *seen.borrow(),
            ["/tmp/a b"],
            "existsSync receives the path as is"
        );
    }

    #[test]
    fn bare_filemanager_is_boolean_and_the_path_is_not_checked() {
        let calls = RefCell::new(0);
        let outcome = validate_input(&args().flag("filemanager", yes()).build(), &|_| {
            *calls.borrow_mut() += 1;
            false
        });
        passes(outcome);
        assert_eq!(
            *calls.borrow(),
            0,
            "existsSync is not called for a boolean value"
        );
    }

    #[test]
    fn q5_numeric_path_is_never_an_existing_path() {
        stops(check(args().flag("filemanager", num(5.0))), NO_PATH, 2);
    }

    #[test]
    fn full_options_set_passes() {
        let argv = args()
            .flag("filemanager", text("."))
            .flag("username", text("bob"))
            .flag("password", text("pw"))
            .flag("role", text("admin"))
            .flag("port", num(18930.0))
            .flag("host", text("127.0.0.1"))
            .flag("public", yes());
        passes(check(argv));
    }
}

mod role {
    use super::*;

    #[test]
    fn unknown_role_stops_and_the_message_ends_with_a_space() {
        let outcome = check(
            args()
                .flag("filemanager", text("."))
                .flag("role", text("root")),
        );
        stops(outcome, BAD_ROLE, 2);
        assert!(
            BAD_ROLE.ends_with(' '),
            "the trailing space is part of the JS message"
        );
    }

    #[test]
    fn role_is_checked_without_filemanager_too() {
        stops(check(args().flag("role", text("root"))), BAD_ROLE, 2);
    }

    #[test]
    fn admin_and_user_pass() {
        for role in ["admin", "user"] {
            passes(check(args().flag("role", text(role))));
        }
    }

    #[test]
    fn role_names_are_case_sensitive() {
        stops(check(args().flag("role", text("Admin"))), BAD_ROLE, 2);
    }

    #[test]
    fn bare_role_is_true_and_stops() {
        stops(check(args().flag("role", yes())), BAD_ROLE, 2);
    }

    #[test]
    fn empty_role_is_falsy_and_passes() {
        passes(check(args().flag("role", text(""))));
    }
}

mod order {
    use super::*;

    #[test]
    fn key_length_comes_before_the_two_strings_check() {
        let argv = args()
            .flag("key", text("abc"))
            .flag("connect", text("k"))
            .pos(text("x"));
        only_reported(argv, KEY_SHORT);
    }

    #[test]
    fn two_strings_come_before_the_live_check() {
        let argv = args()
            .flag("connect", text("k"))
            .pos(text("x"))
            .flag("live", text("abc"));
        only_reported(argv, TWO_STRINGS);
    }

    #[test]
    fn live_comes_before_filemanager_with_udp() {
        let argv = args()
            .flag("live", text("abc"))
            .flag("filemanager", text("/no"))
            .flag("udp", yes());
        only_reported(argv, BAD_PORT);
    }

    #[test]
    fn port_comes_before_filemanager_with_udp() {
        let argv = args()
            .flag("port", text("abc"))
            .flag("filemanager", text("/no"))
            .flag("udp", yes());
        only_reported(argv, BAD_PORT);
    }

    #[test]
    fn filemanager_with_udp_comes_before_filemanager_with_live() {
        let argv = args()
            .flag("filemanager", text("/no"))
            .flag("udp", yes())
            .flag("live", num(3000.0));
        only_reported(argv, FM_UDP);
    }

    #[test]
    fn filemanager_with_live_comes_before_filemanager_with_connect() {
        let argv = args()
            .flag("filemanager", text("/no"))
            .flag("live", num(3000.0))
            .flag("connect", text("k"));
        only_reported(argv, FM_LIVE);
    }

    #[test]
    fn filemanager_with_connect_comes_before_the_path_check() {
        let argv = args()
            .flag("filemanager", text("/no"))
            .flag("connect", text("k"));
        only_reported(argv, FM_CONNECT);
    }

    #[test]
    fn the_path_check_comes_before_the_role_check() {
        let argv = args()
            .flag("filemanager", text("/no"))
            .flag("role", text("root"));
        only_reported(argv, NO_PATH);
    }

    #[test]
    fn q4_exit_zero_beats_every_later_violation() {
        let outcome = check(
            args()
                .flag("key", yes())
                .flag("live", text("abc"))
                .flag("role", text("root")),
        );
        assert_eq!(
            outcome,
            CheckOutcome {
                stdout: red(KEY_EMPTY),
                exit: Some(0)
            }
        );
    }
}

mod matrix {
    use super::*;

    /// All 95 matrix cases through `parse_argv` and `validate_input`: stop, code and printing of the checks as in JS.
    #[test]
    fn every_case_stops_or_passes_exactly_like_js() {
        let Some(cases) = matrix_cases() else {
            panic!("the matrix is not an array")
        };
        assert_eq!(cases.len(), 95, "the matrix must have 95 cases");
        let failures: Vec<String> = cases.iter().filter_map(case_failure).collect();
        assert!(
            failures.is_empty(),
            "differences from JS:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn the_udp_warning_cases_print_the_warning_and_go_on() {
        let cases = matrix_cases().expect("matrix");
        for case in cases.iter().filter(|c| {
            c["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("udp-warn"))
        }) {
            let outcome = run_case(case);
            assert_eq!(
                outcome,
                CheckOutcome {
                    stdout: yellow(UDP_WARN),
                    exit: None
                },
                "case {}",
                case["id"]
            );
        }
    }
}

/// The arguments violate two checks at once (the path does not exist): only the earlier one is printed, code 2.
fn only_reported(argv: Args, message: &str) {
    let outcome = validate_input(&argv.build(), &|_| false);
    assert_eq!(
        outcome,
        CheckOutcome {
            stdout: red(message),
            exit: Some(2)
        }
    );
}

fn matrix_cases() -> Option<Vec<serde_json::Value>> {
    load_value("holesail-cli/cli-matrix.json")
        .as_array()
        .cloned()
}

/// What the checks must print and return for a matrix case (`fs.existsSync` on the testnet: `.` exists, nothing else does).
fn expected(case: &serde_json::Value) -> CheckOutcome {
    let id = case["id"].as_str().expect("id");
    let stdout = case["stdout"].as_str().expect("stdout").to_owned();
    match case["exit"]["code"].as_i64() {
        Some(2) => CheckOutcome {
            stdout,
            exit: Some(2),
        },
        _ if id.starts_with("key-bare") => CheckOutcome {
            stdout,
            exit: Some(0),
        },
        _ if id.starts_with("udp-warn") => CheckOutcome {
            stdout: yellow(UDP_WARN),
            exit: None,
        },
        _ => CheckOutcome {
            stdout: String::new(),
            exit: None,
        },
    }
}

fn run_case(case: &serde_json::Value) -> CheckOutcome {
    let words: Vec<String> = case["args"]
        .as_array()
        .expect("args")
        .iter()
        .map(|a| a.as_str().expect("string").to_owned())
        .collect();
    let parsed: JsValue = parse_argv(words)
        .unwrap_or_else(|e| panic!("case {}: argv parsing failed: {e}", case["id"]));
    validate_input(&parsed, &|path| path == ".")
}

fn case_failure(case: &serde_json::Value) -> Option<String> {
    let (got, want) = (run_case(case), expected(case));
    (got != want).then(|| {
        format!(
            "  {}: {:?}\n    expected {want:?}\n    got {got:?}",
            case["id"], case["args"]
        )
    })
}
