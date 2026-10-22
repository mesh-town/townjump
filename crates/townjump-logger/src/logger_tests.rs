// Tests of `Logger`: line format, stream routing, level filter. Format — docs/spec/holesail/cli.md §7
// and holesail-cli/cli-matrix.json (`live-log`, `live-log0`, `conn-log`); filter — `type < minLevel` from holesail-logger/index.js.
use super::*;
use std::sync::{Arc, Mutex};
use townjump_testkit::vectors::load_value;

mod line_format {
    use super::*;

    #[test]
    fn info_is_green_on_stdout() {
        let f = setup(Some(0.0));
        f.logger.info("hello");
        assert_eq!(
            f.stdout(),
            "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[32m[INFO]\x1b[0m hello\n"
        );
    }

    #[test]
    fn debug_is_grey_on_stdout() {
        let f = setup(Some(0.0));
        f.logger.debug("detail");
        assert_eq!(
            f.stdout(),
            "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[90m[DEBUG]\x1b[0m detail\n"
        );
    }

    #[test]
    fn warn_is_yellow_on_stderr() {
        let f = setup(Some(0.0));
        f.logger.warn("DHT get: no record found");
        let want = "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[33m[WARN]\x1b[0m DHT get: no record found\n";
        assert_eq!(f.stderr(), want);
    }

    #[test]
    fn error_is_red_on_stderr() {
        let f = setup(Some(0.0));
        f.logger.error("failed");
        assert_eq!(
            f.stderr(),
            "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[31m[ERROR]\x1b[0m failed\n"
        );
    }

    #[test]
    fn empty_message_keeps_the_separating_space() {
        let f = setup(Some(0.0));
        f.logger.info("");
        assert_eq!(
            f.stdout(),
            "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[32m[INFO]\x1b[0m \n"
        );
    }

    #[test]
    fn message_is_written_verbatim() {
        let f = setup(Some(0.0));
        f.logger.info("a\nb κλωξ 😀 %s {}");
        assert!(
            f.stdout().ends_with(" a\nb κλωξ 😀 %s {}\n"),
            "got {:?}",
            f.stdout()
        );
    }

    #[test]
    fn timestamp_is_iso_8601_utc_with_milliseconds() {
        let f = setup(Some(0.0));
        f.logger.info("x");
        let raw = f.raw_stdout();
        let stamp = raw
            .strip_prefix("\x1b[90m")
            .and_then(|rest| rest.get(..24))
            .expect("label after \\e[90m");
        assert!(
            is_iso(stamp),
            "label {stamp:?} is not of the form 2026-01-02T03:04:05.678Z"
        );
        assert!(
            raw[5 + 24..].starts_with("\x1b[0m "),
            "label closed by \\e[0m: {raw:?}"
        );
    }
}

mod routing {
    use super::*;

    #[test]
    fn debug_and_info_go_to_stdout_warn_and_error_to_stderr() {
        let f = setup(Some(0.0));
        f.logger.debug("m-debug");
        f.logger.info("m-info");
        f.logger.warn("m-warn");
        f.logger.error("m-error");
        assert_eq!(texts(&f.stdout()), ["m-debug", "m-info"]);
        assert_eq!(texts(&f.stderr()), ["m-warn", "m-error"]);
    }

    #[test]
    fn lines_keep_call_order_within_a_stream() {
        let f = setup(Some(0.0));
        for i in 0..20 {
            f.logger.info(&format!("i{i}"));
            f.logger.warn(&format!("w{i}"));
        }
        let want_info: Vec<String> = (0..20).map(|i| format!("i{i}")).collect();
        let want_warn: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
        assert_eq!(texts(&f.stdout()), want_info);
        assert_eq!(texts(&f.stderr()), want_warn);
    }
}

mod level_filter {
    use super::*;

    #[test]
    fn matches_javascript_comparison_type_less_than_min_level() {
        for (level, shown) in LEVELS {
            let f = setup(*level);
            f.logger.debug("m-debug");
            f.logger.info("m-info");
            f.logger.warn("m-warn");
            f.logger.error("m-error");
            let seen = format!("{}{}", f.stdout(), f.stderr());
            for (name, want) in ["m-debug", "m-info", "m-warn", "m-error"].iter().zip(shown) {
                assert_eq!(seen.contains(name), *want, "level {level:?}: {name}");
            }
        }
    }

    #[test]
    fn disabled_logger_prints_nothing() {
        let f = setup(None);
        f.logger.error("fatal");
        assert_eq!((f.stdout(), f.stderr()), (String::new(), String::new()));
    }
}

mod clones {
    use super::*;

    #[test]
    fn share_outputs_with_the_original() {
        let f = setup(Some(0.0));
        let copy = f.logger.clone();
        f.logger.info("from original");
        copy.info("from clone");
        copy.warn("clone warning");
        assert_eq!(texts(&f.stdout()), ["from original", "from clone"]);
        assert_eq!(texts(&f.stderr()), ["clone warning"]);
    }

    #[test]
    fn keep_the_level_of_the_original() {
        let f = setup(Some(2.0));
        let copy = f.logger.clone();
        copy.info("hidden");
        copy.warn("shown");
        assert_eq!(texts(&f.stdout()), Vec::<String>::new());
        assert_eq!(texts(&f.stderr()), ["shown"]);
    }
}

mod real_cli_output {
    use super::*;

    #[test]
    fn logger_reproduces_the_log_lines_of_the_js_cli() {
        let streams = [
            ("live-log", "stdout"),
            ("live-log0", "stdout"),
            ("conn-log", "stdout"),
            ("conn-log", "stderr"),
        ];
        for (id, stream) in streams {
            let want = log_lines(id, stream);
            assert!(
                !want.is_empty(),
                "cli-matrix.json {id}/{stream} has no log lines"
            );
            let f = setup(Some(0.0));
            want.iter().for_each(|line| replay(&f.logger, line));
            let got = if stream == "stdout" {
                f.stdout()
            } else {
                f.stderr()
            };
            assert_eq!(got, mask_timestamps(&want.concat()), "{id}/{stream}");
        }
    }
}

mod threads {
    use super::*;

    #[test]
    fn lines_from_many_threads_stay_whole_and_ordered_per_thread() {
        let f = setup(Some(0.0));
        let workers: Vec<_> = (0..8)
            .map(|t| {
                let logger = f.logger.clone();
                std::thread::spawn(move || (0..50).for_each(|i| logger.info(&format!("t{t}-{i}"))))
            })
            .collect();
        workers.into_iter().for_each(|w| w.join().expect("thread"));
        let head = "\x1b[90m<TS>\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[32m[INFO]\x1b[0m t";
        let out = f.stdout();
        assert!(
            out.lines().all(|l| l.starts_with(head)),
            "line is interleaved: {out:?}"
        );
        let got = texts(&out);
        assert_eq!(got.len(), 400, "all lines arrived");
        for t in 0..8 {
            let own: Vec<&String> = got
                .iter()
                .filter(|m| m.starts_with(&format!("t{t}-")))
                .collect();
            let want: Vec<String> = (0..50).map(|i| format!("t{t}-{i}")).collect();
            assert_eq!(own, want.iter().collect::<Vec<_>>(), "stream order {t}");
        }
    }
}

// Which of DEBUG(0), INFO(1), WARN(2), ERROR(3) are printed at `minLevel`: printed if `type >= minLevel`
// or the comparison is false (NaN). `None` — the logger is off.
const LEVELS: &[(Option<f64>, [bool; 4])] = &[
    (None, [false; 4]),
    (Some(0.0), [true; 4]),
    (Some(1.0), [false, true, true, true]),
    (Some(2.0), [false, false, true, true]),
    (Some(3.0), [false, false, false, true]),
    (Some(4.0), [false; 4]),
    (Some(0.5), [false, true, true, true]),
    (Some(2.5), [false, false, false, true]),
    (Some(3.5), [false; 4]),
    (Some(-1.0), [true; 4]),
    (Some(f64::NAN), [true; 4]),
    (Some(f64::INFINITY), [false; 4]),
    (Some(f64::NEG_INFINITY), [true; 4]),
];

struct Fixture {
    logger: Logger,
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
}

impl Fixture {
    fn raw_stdout(&self) -> String {
        String::from_utf8(self.out.lock().expect("stdout").clone()).expect("utf-8")
    }

    /// stdout with timestamps replaced by `<TS>`.
    fn stdout(&self) -> String {
        mask_timestamps(&self.raw_stdout())
    }

    fn stderr(&self) -> String {
        mask_timestamps(
            &String::from_utf8(self.err.lock().expect("stderr").clone()).expect("utf-8"),
        )
    }
}

fn setup(level: Option<f64>) -> Fixture {
    let out = Arc::new(Mutex::new(Vec::new()));
    let err = Arc::new(Mutex::new(Vec::new()));
    let logger = Logger::new(level, out.clone(), err.clone());
    Fixture { logger, out, err }
}

/// Replaces `\e[90m<ISO-stamp>` at line starts with `\e[90m<TS>`; a line with a wrong stamp stays as is and will not match the expectation.
fn mask_timestamps(text: &str) -> String {
    let mask = |line: &str| match line
        .strip_prefix("\x1b[90m")
        .and_then(|rest| rest.split_at_checked(24))
    {
        Some((stamp, tail)) if is_iso(stamp) => format!("\x1b[90m<TS>{tail}"),
        _ => line.to_string(),
    };
    text.split_inclusive('\n').map(mask).collect()
}

fn is_iso(stamp: &str) -> bool {
    let shape = "dddd-dd-ddTdd:dd:dd.dddZ";
    stamp.len() == shape.len()
        && shape
            .chars()
            .zip(stamp.chars())
            .all(|(p, c)| if p == 'd' { c.is_ascii_digit() } else { p == c })
}

/// Message texts from log lines: everything after `[LEVEL]\e[0m `.
fn texts(stream: &str) -> Vec<String> {
    let text_of = |line: &str| {
        line.split_once("\x1b[0m ")
            .and_then(|(_, r)| r.split_once("\x1b[0m "))
            .and_then(|(_, r)| r.split_once("\x1b[0m "))
            .map(|(_, m)| m.trim_end().to_string())
    };
    stream.lines().filter_map(text_of).collect()
}

/// Log lines (with the `[Holesail]` tag) from stream `stream` of case `id` in cli-matrix.json, with `\n`.
fn log_lines(id: &str, stream: &str) -> Vec<String> {
    let matrix = load_value("holesail-cli/cli-matrix.json");
    let cases = matrix
        .get("cases")
        .unwrap_or(&matrix)
        .as_array()
        .expect("cases");
    let case = cases
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no case {id}"));
    let text = case[stream].as_str().expect("stream is a string");
    let is_log = |l: &&str| l.contains("\x1b[34m[Holesail]\x1b[0m");
    text.split_inclusive('\n')
        .filter(is_log)
        .map(String::from)
        .collect()
}

/// Prints the log line's message with the method whose level is in the line.
fn replay(logger: &Logger, line: &str) {
    let msg = texts(line).pop().expect("message in the line");
    match ["[DEBUG]", "[INFO]", "[WARN]", "[ERROR]"]
        .iter()
        .position(|l| line.contains(l))
    {
        Some(0) => logger.debug(&msg),
        Some(1) => logger.info(&msg),
        Some(2) => logger.warn(&msg),
        Some(3) => logger.error(&msg),
        _ => panic!("no level in line {line:?}"),
    }
}
