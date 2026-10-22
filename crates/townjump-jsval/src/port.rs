// JS: node:net `validatePort` (lib/internal/validators.js), called from `net.connect`/`listen`.
// Related: value.rs (input is `JsValue` from minimist), error.rs (`RangeError` with code ERR_SOCKET_BAD_PORT).
use crate::{
    error::JsError,
    number::number_to_string,
    string_number::{string_to_number, trim_js},
    value::JsValue,
};

/// Port as `net.connect` accepts it: an integer 0..65535 (number or numeric string); an out-of-range number, a fraction, NaN and
/// a non-numeric string give `RangeError` ERR_SOCKET_BAD_PORT; boolean, null, array, object give `TypeError` ERR_INVALID_ARG_TYPE (the type
/// is checked before `validatePort`). The caller applies unary `+` before the call where JS does (`pipeTcpServer`).
pub fn to_port(v: &JsValue) -> Result<u16, JsError> {
    match v {
        JsValue::Number(n) => check(*n, || format!("type number ({})", number_to_string(*n))),
        JsValue::String(s) => check(string_to_number(s), || format!("type string ('{s}')"))
            .and_then(|p| non_blank(s).map(|()| p)),
        other => Err(wrong_type(other)),
    }
}

fn check(n: f64, shown: impl FnOnce() -> String) -> Result<u16, JsError> {
    let integral = n.fract() == 0.0 && (0.0..=65535.0).contains(&n);
    if integral {
        return Ok(n as u16);
    }
    Err(JsError::Range {
        code: "ERR_SOCKET_BAD_PORT".into(),
        message: format!("Port should be >= 0 and < 65536. Received {}.", shown()),
    })
}

/// A string that is empty after `trim` is rejected before the numeric comparison (`+'' === 0` would let it through).
fn non_blank(s: &str) -> Result<(), JsError> {
    match trim_js(s).is_empty() {
        true => check(f64::NAN, || format!("type string ('{s}')")).map(|_| ()),
        false => Ok(()),
    }
}

fn wrong_type(v: &JsValue) -> JsError {
    let received = match v {
        JsValue::Bool(b) => format!("type boolean ({b})"),
        JsValue::Null => "null".into(),
        JsValue::Array(_) => "an instance of Array".into(),
        _ => "an instance of Object".into(),
    };
    JsError::Type(format!(
        "The \"options.port\" property must be one of type number or string. Received {received}"
    ))
}

#[cfg(test)]
#[path = "port_tests.rs"]
mod tests;
