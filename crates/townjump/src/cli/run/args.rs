// JS: holesail/src/bin/holesail.mjs (dispatcher branch choice, `--log` parsing, building the Holesail options).
// See also: run.rs calls `Mode::of`, serve.rs — `log_level`, `server_opts`, `client_opts`.
use crate::facade::HolesailOpts;
use townjump_jsval::JsValue;

/// Dispatcher branch.
pub(super) enum Mode {
    Help,
    Version,
    Live,
    Connect,
    Filemanager,
    Lookup,
    Fallback,
}

impl Mode {
    /// Order from cli.md, section 4.
    pub(super) fn of(args: &JsValue) -> Mode {
        let has = |name: &str| flag(args, name).is_some_and(JsValue::truthy);
        let modes = [
            (has("help") || has("h"), Mode::Help),
            (has("version"), Mode::Version),
            (has("live"), Mode::Live),
            (has("connect") || first_positional(args), Mode::Connect),
            (has("filemanager"), Mode::Filemanager),
            (has("lookup"), Mode::Lookup),
        ];
        let hit = modes.into_iter().find(|(hit, _)| *hit);
        hit.map_or(Mode::Fallback, |(_, mode)| mode)
    }
}

pub(super) fn flag<'a>(args: &'a JsValue, name: &str) -> Option<&'a JsValue> {
    args.get(name).ok().flatten()
}

fn first_positional(args: &JsValue) -> bool {
    let Some(JsValue::Array(items)) = flag(args, "_") else {
        return false;
    };
    items.first().is_some_and(JsValue::truthy)
}

/// `argv.log` → logger level: `None` — logging off.
pub(super) fn log_level(args: &JsValue) -> Option<f64> {
    match flag(args, "log")? {
        JsValue::Bool(true) => Some(1.0),
        JsValue::String(text) => Some(clamp(parse_int(text).unwrap_or(1.0))),
        JsValue::Number(n) => Some(clamp(*n)),
        _ => None,
    }
}

fn clamp(level: f64) -> f64 {
    level.clamp(0.0, 3.0)
}

/// `parseInt(text, 10)`: spaces, sign, decimal digits; otherwise `NaN` (`None`).
fn parse_int(text: &str) -> Option<f64> {
    let text = text.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let number: f64 = digits[..end].parse().ok()?;
    Some(sign * number)
}

/// `new Holesail({ server: true, port: argv.live, host: argv.host || '127.0.0.1', udp, secure, key })`.
pub(super) fn server_opts(args: &JsValue) -> HolesailOpts {
    let host = flag(args, "host").filter(|h| h.truthy());
    let secure = flag(args, "public").is_none_or(|p| !p.truthy());
    HolesailOpts {
        server: true,
        port: flag(args, "live").cloned(),
        host: Some(
            host.cloned()
                .unwrap_or_else(|| JsValue::String("127.0.0.1".into())),
        ),
        key: flag(args, "key").cloned(),
        udp: flag(args, "udp").cloned(),
        secure,
    }
}

/// `new Holesail({ server: false, key: argv.connect || argv._[0], port, host, udp, secure: argv.public })`.
pub(super) fn client_opts(args: &JsValue) -> HolesailOpts {
    let connect = flag(args, "connect").filter(|c| c.truthy());
    let positional = match flag(args, "_") {
        Some(JsValue::Array(items)) => items.first(),
        _ => None,
    };
    HolesailOpts {
        server: false,
        port: flag(args, "port").cloned(),
        host: flag(args, "host").cloned(),
        key: connect.or(positional).cloned(),
        udp: flag(args, "udp").cloned(),
        secure: flag(args, "public").is_some_and(JsValue::truthy),
    }
}
