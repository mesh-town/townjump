// JS: minimist/index.js:setKey, setArg, isNumber (storing values and nested keys `a.b.c`).
// See also: argv.rs decides what each token means; here only the result storage.
use townjump_jsval::{JsError, JsValue};

type Pairs = Vec<(String, JsValue)>;

/// Names from `Object.prototype`: `aliases[key]` yields a function that has no `forEach` (a minimist quirk).
const PROTOTYPE_NAMES: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

/// Parse result: an object with the `_` key first.
pub(super) struct Store {
    pairs: Pairs,
}

impl Store {
    pub(super) fn new() -> Store {
        Store {
            pairs: vec![("_".to_owned(), JsValue::Array(Vec::new()))],
        }
    }

    pub(super) fn finish(self) -> JsValue {
        JsValue::Object(self.pairs)
    }

    /// Positional argument: a number-like one becomes a number.
    pub(super) fn positional(&mut self, word: &str) {
        self.push_positional(text_value(word));
    }

    /// The tail after `--` goes as strings, unchanged.
    pub(super) fn positional_raw(&mut self, word: &str) {
        self.push_positional(JsValue::String(word.to_owned()));
    }

    /// `setArg(key, text)`: a number-like string is turned into a number.
    pub(super) fn set_text(&mut self, key: &str, text: &str) -> Result<(), JsError> {
        self.set(key, text_value(text))
    }

    pub(super) fn set_bool(&mut self, key: &str, flag: bool) -> Result<(), JsError> {
        self.set(key, JsValue::Bool(flag))
    }

    fn set(&mut self, key: &str, value: JsValue) -> Result<(), JsError> {
        if PROTOTYPE_NAMES.contains(&key) {
            return Err(JsError::Type(
                "(aliases[key] || []).forEach is not a function".to_owned(),
            ));
        }
        let path: Vec<&str> = key.split('.').collect();
        set_key(&mut self.pairs, &path, value)
    }

    fn push_positional(&mut self, value: JsValue) {
        if let Some((_, JsValue::Array(items))) = self.pairs.first_mut() {
            items.push(value);
        }
    }
}

fn text_value(text: &str) -> JsValue {
    if is_number(text) {
        return JsValue::Number(to_number(text));
    }
    JsValue::String(text.to_owned())
}

/// `setKey`: a path of nested objects; `constructor` and `__proto__` are silently dropped.
fn set_key(obj: &mut Pairs, path: &[&str], value: JsValue) -> Result<(), JsError> {
    let (key, rest) = path.split_first().expect("split yields at least one key");
    if PROTOTYPE_NAMES.contains(key) {
        return Ok(());
    }
    if rest.is_empty() {
        assign(obj, key, value);
        return Ok(());
    }
    match slot(obj, key) {
        JsValue::Object(inner) => set_key(inner, rest, value),
        JsValue::Array(_) => Ok(()),
        scalar => Err(scalar_error(scalar, rest[0])),
    }
}

fn slot<'a>(obj: &'a mut Pairs, key: &str) -> &'a mut JsValue {
    let at = obj.iter().position(|(k, _)| k == key).unwrap_or_else(|| {
        obj.push((key.to_owned(), JsValue::Object(Vec::new())));
        obj.len() - 1
    });
    &mut obj[at].1
}

/// Last key: a new value, replacing a boolean, appending to an array or turning into an array.
fn assign(obj: &mut Pairs, key: &str, value: JsValue) {
    let Some((_, old)) = obj.iter_mut().find(|(k, _)| k == key) else {
        obj.push((key.to_owned(), value));
        return;
    };
    match old {
        JsValue::Bool(_) => *old = value,
        JsValue::Array(items) => items.push(value),
        _ => *old = JsValue::Array(vec![std::mem::replace(old, JsValue::Null), value]),
    }
}

/// V8 strict-mode TypeError: writing a property on a primitive.
fn scalar_error(scalar: &JsValue, key: &str) -> JsError {
    let kind = match scalar {
        JsValue::Number(_) => "number",
        JsValue::Bool(_) => "boolean",
        _ => "string",
    };
    let shown = scalar.to_js_string();
    JsError::Type(format!(
        "Cannot create property '{key}' on {kind} '{shown}'"
    ))
}

/// `isNumber`: `^0x[0-9a-f]+$` (case-insensitive) or a decimal literal with a lowercase exponent `e`.
fn is_number(s: &str) -> bool {
    hex_digits(s).is_some() || is_decimal(s)
}

fn hex_digits(s: &str) -> Option<&str> {
    let digits = s.get(..2).filter(|p| p.eq_ignore_ascii_case("0x"))?;
    let digits = &s[digits.len()..];
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digits)
}

fn is_decimal(s: &str) -> bool {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exponent) = s.split_once('e').map_or((s, None), |(m, e)| (m, Some(e)));
    is_mantissa(mantissa) && exponent.is_none_or(is_exponent)
}

fn is_mantissa(m: &str) -> bool {
    let (int, frac) = m.split_once('.').map_or((m, None), |(i, f)| (i, Some(f)));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let has_digit = !int.is_empty() || frac.is_some_and(|f| !f.is_empty());
    digits(int) && frac.is_none_or(digits) && has_digit
}

fn is_exponent(e: &str) -> bool {
    let e = e.strip_prefix('-').or(e.strip_prefix('+')).unwrap_or(e);
    !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
}

/// `Number(s)` for a string that passed `is_number`.
fn to_number(s: &str) -> f64 {
    match hex_digits(s) {
        Some(digits) => digits.chars().fold(0.0, |acc, c| {
            acc * 16.0 + f64::from(c.to_digit(16).unwrap_or(0))
        }),
        None => s.parse().unwrap_or(f64::NAN),
    }
}
