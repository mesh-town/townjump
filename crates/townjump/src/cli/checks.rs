// JS: holesail/src/lib/validateInput.js:validateInput (11 checks in the order of cli.md, section 3).
// See also: run.rs calls it before help and version; the result is printed to stdout.
use townjump_jsval::JsValue;

/// Check outcome: text for stdout (with ANSI) and the exit code if the run is stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    /// Warnings and the error text; may be non-empty even with `exit: None` (the UDP warning).
    pub stdout: String,
    /// `None` — continue; `Some(0)` — `--key` without a value (as in JS); `Some(2)` — input error.
    pub exit: Option<i32>,
}

/// What one check decided.
enum Outcome {
    Pass,
    /// Yellow text, the run continues.
    Warn(&'static str),
    /// Red text and the exit code.
    Stop(&'static str, i32),
}

/// Arguments and `fs.existsSync` for the checks.
struct Input<'a> {
    argv: &'a JsValue,
    path_exists: &'a dyn Fn(&str) -> bool,
}

type Check = fn(&Input) -> Outcome;

/// Order from validateInput.js.
const CHECKS: [Check; 11] = [
    key_is_bare,
    udp_host_warning,
    key_too_short,
    two_connection_strings,
    live_not_number,
    port_not_number,
    filemanager_with_udp,
    live_with_filemanager,
    filemanager_with_connection,
    filemanager_path_missing,
    role_unknown,
];

/// `path_exists` — `fs.existsSync` (substituted in tests).
pub fn validate_input(argv: &JsValue, path_exists: &dyn Fn(&str) -> bool) -> CheckOutcome {
    let input = Input { argv, path_exists };
    let mut stdout = String::new();
    for check in CHECKS {
        match check(&input) {
            Outcome::Pass => {}
            Outcome::Warn(text) => stdout.push_str(&colored("33", text)),
            Outcome::Stop(text, code) => {
                stdout.push_str(&colored("31", text));
                return CheckOutcome {
                    stdout,
                    exit: Some(code),
                };
            }
        }
    }
    CheckOutcome { stdout, exit: None }
}

/// `barely-colours` and `console.log`.
fn colored(code: &str, text: &str) -> String {
    format!("\u{1b}[{code}m{text}\u{1b}[0m\n")
}

impl Input<'_> {
    fn flag(&self, name: &str) -> Option<&JsValue> {
        self.argv.get(name).ok().flatten()
    }

    fn truthy(&self, name: &str) -> bool {
        self.flag(name).is_some_and(JsValue::truthy)
    }

    /// `argv._[0]` is truthy.
    fn first_positional(&self) -> bool {
        let Some(JsValue::Array(items)) = self.flag("_") else {
            return false;
        };
        items.first().is_some_and(JsValue::truthy)
    }

    fn has_connection(&self) -> bool {
        self.truthy("connect") || self.first_positional()
    }
}

fn stop_if(condition: bool, text: &'static str, code: i32) -> Outcome {
    if condition {
        return Outcome::Stop(text, code);
    }
    Outcome::Pass
}

fn is_number(value: Option<&JsValue>) -> bool {
    matches!(value, Some(JsValue::Number(_)))
}

fn key_is_bare(input: &Input) -> Outcome {
    let bare = matches!(input.flag("key"), Some(JsValue::Bool(true)));
    stop_if(bare, "Error: Key can not be empty", 0)
}

fn udp_host_warning(input: &Input) -> Outcome {
    let host = input.flag("host");
    let local = matches!(host, Some(JsValue::String(h)) if h == "localhost" || h == "0.0.0.0");
    if input.truthy("udp") && local {
        return Outcome::Warn(
            "Warning: localhost or 0.0.0.0 may not work properly within netcat with UDP.",
        );
    }
    Outcome::Pass
}

/// `args.key.length < 32`: a string and an array have `length`, a number does not (a JS quirk).
fn key_too_short(input: &Input) -> Outcome {
    let length = match input.flag("key") {
        Some(JsValue::String(key)) => Some(key.encode_utf16().count()),
        Some(JsValue::Array(items)) => Some(items.len()),
        _ => None,
    };
    let short = input.truthy("key") && length.is_some_and(|n| n < 32) && !input.truthy("force");
    stop_if(
        short,
        "Error: A key should have a minimum length of 32 chars for security purposes. If you still wish to proceed use --force",
        2,
    )
}

fn two_connection_strings(input: &Input) -> Outcome {
    let both = input.truthy("connect") && input.first_positional();
    stop_if(
        both,
        "Error: Are you trying to use two connection strings at once? Get some holesail --help",
        2,
    )
}

const BAD_PORT: &str =
    "Error: Given port is not a valid number. Run holesail --help to see examples";

fn live_not_number(input: &Input) -> Outcome {
    let bad = input.truthy("live") && !is_number(input.flag("live"));
    stop_if(bad, BAD_PORT, 2)
}

/// A truthy non-number or an empty string (`--port=`); `--port 0` passes.
fn port_not_number(input: &Input) -> Outcome {
    let port = input.flag("port");
    let empty = matches!(port, Some(JsValue::String(p)) if p.is_empty());
    let bad = (input.truthy("port") && !is_number(port)) || empty;
    stop_if(bad, BAD_PORT, 2)
}

fn filemanager_with_udp(input: &Input) -> Outcome {
    let both = input.truthy("filemanager") && input.truthy("udp");
    stop_if(both, "Error: You can't run filemanager in UDP mode.", 2)
}

fn live_with_filemanager(input: &Input) -> Outcome {
    let both = input.truthy("live") && input.truthy("filemanager");
    stop_if(
        both,
        "Error: You can't start holesail server and filemanager at the same time. If you are trying to use filemanager on a specific local port use --port instead or see holesail --help",
        2,
    )
}

fn filemanager_with_connection(input: &Input) -> Outcome {
    let both = input.truthy("filemanager") && input.has_connection();
    stop_if(
        both,
        "Error: You tried to create a connection and start filemanager both at once. Start them separately and check your command for mistakes. See holesail --help",
        2,
    )
}

/// `fs.existsSync(path)`: a non-string (number, array) gives `false`.
fn filemanager_path_missing(input: &Input) -> Outcome {
    let missing = match input.flag("filemanager") {
        Some(JsValue::Bool(_)) | None => false,
        Some(JsValue::String(path)) => !(input.path_exists)(path),
        Some(other) => other.truthy(),
    };
    stop_if(missing, "Error: Given path does not exist", 2)
}

fn role_unknown(input: &Input) -> Outcome {
    let known =
        matches!(input.flag("role"), Some(JsValue::String(r)) if r == "admin" || r == "user");
    let bad = input.truthy("role") && !known;
    stop_if(
        bad,
        "Error: Incorrect role set. Role can be either \"admin\" or \"user\" ",
        2,
    )
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "checks_flow_tests.rs"]
mod flow_tests;

#[cfg(test)]
#[path = "checks_ports_tests.rs"]
mod ports_tests;
