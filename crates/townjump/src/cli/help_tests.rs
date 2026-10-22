// Tests of `cli::help`: help and version byte for byte against golden/*.txt and the CLI matrix stdout (cases help-*, version*).
// `help_text` and `version_text` are exactly the JS stdout: they stand in the `cli-matrix.json` vectors (`help-noargs` ends with `\n\n`, `version` is `2.4.4\n`).
use super::*;
use townjump_jsval::JsValue;
use townjump_testkit::vectors::load_value;

mod general {
    use super::*;

    #[test]
    fn no_topic_prints_the_general_help() {
        assert_eq!(help_text(None), golden("help-noargs"));
    }

    #[test]
    fn unknown_word_prints_the_general_help() {
        assert_eq!(help_text(Some(&text("foo"))), golden("help-noargs"));
    }

    #[test]
    fn number_topic_prints_the_general_help() {
        assert_eq!(
            help_text(Some(&JsValue::Number(3000.0))),
            golden("help-noargs")
        );
    }

    #[test]
    fn true_topic_from_a_bare_help_flag_prints_the_general_help() {
        assert_eq!(help_text(Some(&JsValue::Bool(true))), golden("help-noargs"));
    }

    #[test]
    fn topic_names_are_case_sensitive() {
        assert_eq!(help_text(Some(&text("LIVE"))), golden("help-noargs"));
    }

    #[test]
    fn empty_string_topic_prints_the_general_help() {
        assert_eq!(help_text(Some(&text(""))), golden("help-noargs"));
    }

    #[test]
    fn repeated_topic_is_an_array_and_prints_the_general_help() {
        let topics = JsValue::Array(vec![text("live"), text("connect")]);
        assert_eq!(help_text(Some(&topics)), golden("help-noargs"));
    }

    #[test]
    fn it_still_advertises_dash_v_although_q3_says_it_does_nothing() {
        assert!(
            help_text(None).contains("--version|-v"),
            "the help promises `-v`, which JS does not handle"
        );
    }

    #[test]
    fn trailing_spaces_and_the_two_space_line_are_kept() {
        let help = help_text(None);
        assert!(
            help.contains("Start a filemanager session.  \n"),
            "two spaces at the end of the line"
        );
        assert!(
            help.contains("\n\n  \n\u{1b}[1mCommands:"),
            "a line of two spaces before Commands"
        );
        assert!(
            help.ends_with("connectors.\n\n"),
            "console.log adds a second line break"
        );
    }
}

mod topics {
    use super::*;

    #[test]
    fn live_topic_prints_the_live_help() {
        assert_eq!(help_text(Some(&text("live"))), golden("help-live"));
    }

    #[test]
    fn connect_topic_prints_the_connect_help() {
        assert_eq!(help_text(Some(&text("connect"))), golden("help-connect"));
    }

    #[test]
    fn filemanager_topic_prints_the_filemanager_help() {
        assert_eq!(
            help_text(Some(&text("filemanager"))),
            with_port_lines(&golden("help-filemanager"))
        );
    }

    #[test]
    fn filemanager_help_adds_the_safe_mode_line_after_role() {
        let js = golden("help-filemanager");
        assert_eq!(include_str!("help_text/filemanager.txt"), js);
        let out = help_text(Some(&text("filemanager")));
        assert_eq!(out.len(), js.len() + SAFE_MODE_LINE.len());
        let lines: Vec<&str> = out.split_inclusive('\n').collect();
        let at = lines
            .iter()
            .position(|l| l.contains("--role <admin|user>"))
            .expect("the --role line");
        assert_eq!(lines[at + 1], SAFE_MODE_LINE);
    }

    #[test]
    fn subhelp_keeps_the_trailing_spaces_of_js() {
        let live = help_text(Some(&text("live")));
        assert!(
            live.contains("Reverse proxy a specific port\u{1b}[0m    \n"),
            "four spaces after the header"
        );
        assert!(
            live.ends_with("Set a custom key.\n    \n"),
            "tail `\\n    ` and `\\n` from console.log"
        );
    }
}

mod version {
    use super::*;

    #[test]
    fn version_text_is_the_golden_bytes() {
        assert_eq!(version_text(), golden("version"));
    }

    #[test]
    fn version_text_is_the_constant_and_a_newline() {
        assert_eq!(version_text(), format!("{VERSION}\n"));
        assert_eq!(version_text(), "2.4.4\n");
    }
}

mod matrix {
    use super::*;

    /// The `help-*` matrix cases: the topic is what `printHelp` receives as `argv.help`.
    #[test]
    fn help_cases_print_the_bytes_js_printed() {
        let cases: [(&str, Option<JsValue>); 7] = [
            ("help-noargs", None),
            ("help-flag", Some(JsValue::Bool(true))),
            ("help-live", Some(text("live"))),
            ("help-connect", Some(text("connect"))),
            ("help-filemanager", Some(text("filemanager"))),
            ("help-unknown", Some(text("foo"))),
            ("help-short-arg", None),
        ];
        for (id, topic) in cases {
            let expected = match id {
                "help-filemanager" => with_port_lines(&matrix_stdout(id)),
                _ => matrix_stdout(id),
            };
            assert_eq!(help_text(topic.as_ref()), expected, "case {id}");
        }
    }

    #[test]
    fn unknown_flags_and_dash_v_fall_back_to_the_general_help() {
        for id in [
            "version-short",
            "unknown-flag",
            "unknown-positional-only-flag-arg",
            "live-zero",
        ] {
            assert_eq!(
                help_text(None),
                matrix_stdout(id),
                "case {id} prints the general help"
            );
        }
    }

    #[test]
    fn version_cases_print_the_version() {
        for id in ["version", "version-before-live"] {
            assert_eq!(version_text(), matrix_stdout(id), "case {id}");
        }
    }
}

fn text(s: &str) -> JsValue {
    JsValue::String(s.to_owned())
}

fn golden(name: &str) -> String {
    let path = format!(
        "{}/../../testvectors/holesail-cli/golden/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("no reference {path}: {e}"))
}

fn matrix_stdout(id: &str) -> String {
    let matrix = load_value("holesail-cli/cli-matrix.json");
    let case = matrix
        .as_array()
        .and_then(|all| all.iter().find(|c| c["id"] == id));
    case.unwrap_or_else(|| panic!("no case {id} in the matrix"))["stdout"]
        .as_str()
        .expect("stdout is a string")
        .to_owned()
}
