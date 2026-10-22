// JS: holesail/src/bin/holesail.mjs (the `--lookup` branch): `Holesail.lookup`, four result lines, an error exits with code 0.
// See also: facade/lookup.rs provides the result object.
use super::args::flag;
use super::{CliIo, emit};
use crate::facade::Holesail;
use townjump_dht::DhtFactory;
use townjump_jsval::JsValue;

const TITLE: &str =
    "\u{1b}[36m\u{1b}[4m\u{1b}[1mHolesail Lookup Result\u{1b}[0m\u{1b}[0m\u{1b}[0m 🔍";

pub(super) async fn run<F: DhtFactory>(args: &JsValue, io: &CliIo, factory: &F) -> i32 {
    let url = flag(args, "lookup")
        .map(JsValue::to_js_string)
        .unwrap_or_default();
    match Holesail::lookup(factory, &url).await {
        Ok(found) => emit(&io.stdout, &report(&found)),
        Err(error) => emit(&io.stderr, &failure(&error.to_string())),
    }
    0
}

fn failure(message: &str) -> String {
    format!("\u{1b}[31mError during lookup:\u{1b}[0m {message}\n")
}

fn report(found: &JsValue) -> String {
    let protocol = shown(found, "protocol").to_uppercase();
    let private = if field(found, "secure").is_some_and(JsValue::truthy) {
        "Yes"
    } else {
        "No"
    };
    let rows = [
        ("Host", shown(found, "host")),
        ("Port", shown(found, "port")),
        ("Protocol", protocol),
        ("Private", private.to_owned()),
    ];
    let lines = rows.map(|(label, value)| row(label, &value));
    format!("{TITLE}\n{}", lines.concat())
}

fn row(label: &str, value: &str) -> String {
    format!("\u{1b}[35m{label}: \u{1b}[0m\u{1b}[32m{value}\u{1b}[0m\n")
}

fn field<'a>(found: &'a JsValue, name: &str) -> Option<&'a JsValue> {
    found.get(name).ok().flatten()
}

/// `value || 'N/A'`.
fn shown(found: &JsValue, name: &str) -> String {
    match field(found, name).filter(|v| v.truthy()) {
        Some(value) => value.to_js_string(),
        None => "N/A".to_owned(),
    }
}
