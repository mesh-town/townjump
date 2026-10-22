// JS: holesail/src/lib/help.js:printHelp, version from package.json (`2.4.4`).
// See also: run.rs prints the result; the byte reference is testvectors/holesail-cli/golden/.

use townjump_jsval::JsValue;

// The texts are the JS output byte for byte (ANSI, trailing spaces, the final `\n` from `console.log`).
const GENERAL: &str = include_str!("help_text/noargs.txt");
const LIVE: &str = include_str!("help_text/live.txt");
const CONNECT: &str = include_str!("help_text/connect.txt");
const FILEMANAGER: &str = include_str!("help_text/filemanager.txt");

/// CLI version.
pub const VERSION: &str = "2.4.4";

/// Help text for `--help <topic>`; topic `live`, `connect`, `filemanager`, otherwise the general one (including a number, `true`).
pub fn help_text(topic: Option<&townjump_jsval::JsValue>) -> String {
    match topic {
        Some(JsValue::String(t)) if t == "live" => LIVE,
        Some(JsValue::String(t)) if t == "connect" => CONNECT,
        Some(JsValue::String(t)) if t == "filemanager" => return with_port_lines(FILEMANAGER),
        _ => GENERAL,
    }
    .to_owned()
}

/// The help line about `--safe-mode` (a port extension; absent in JS).
pub const SAFE_MODE_LINE: &str = "  \x1b[33m--safe-mode\x1b[0m        Harden the file server (needs --username and --password).\n";

/// JS help plus the port lines: `SAFE_MODE_LINE` right after the `--role <admin|user>` line.
pub fn with_port_lines(js: &str) -> String {
    let mut out = String::with_capacity(js.len() + SAFE_MODE_LINE.len());
    for line in js.split_inclusive('\n') {
        out.push_str(line);
        if line.contains("--role <admin|user>") {
            out.push_str(SAFE_MODE_LINE);
        }
    }
    out
}

/// Output of `--version`: `2.4.4\n`.
pub fn version_text() -> String {
    format!("{VERSION}\n")
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod tests;
