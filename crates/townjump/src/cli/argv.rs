// JS: minimist 1.2.8 (index.js), call `minimist(process.argv.slice(2))` without options.
// See also: checks.rs and run.rs read the result as `JsValue::Object`; rules — cli.md, section 1; store.rs — value storage.
use store::Store;
use townjump_jsval::{JsError, JsValue};

mod store;

/// What kind a token is (minimist check order).
enum Kind<'a> {
    /// `--key=value`.
    Assign(&'a str, &'a str),
    /// `--no-key`.
    Negated(&'a str),
    /// `--key [value]`.
    Long(&'a str),
    /// `-abc [value]`.
    Short,
    Positional,
}

/// argv parsing: an object with the `_` key (positionals) first; `--__proto__ x` — `TypeError`.
pub fn parse_argv(argv: Vec<String>) -> Result<JsValue, JsError> {
    let cut = argv.iter().position(|w| w == "--").unwrap_or(argv.len());
    let mut store = Store::new();
    let mut at = 0;
    while at < cut {
        at += consume(&mut store, &argv[..cut], at)?;
    }
    argv.iter()
        .skip(cut + 1)
        .for_each(|w| store.positional_raw(w));
    Ok(store.finish())
}

/// Handles the token at `at`; returns how many tokens were consumed (1 or 2).
fn consume(store: &mut Store, args: &[String], at: usize) -> Result<usize, JsError> {
    let arg = args[at].as_str();
    let next = args.get(at + 1).map(String::as_str);
    match classify(arg)? {
        Kind::Assign(key, value) => store.set_text(key, value).map(|()| 1),
        Kind::Negated(key) => store.set_bool(key, false).map(|()| 1),
        Kind::Long(key) => long_flag(store, key, next),
        Kind::Short => short_flag(store, arg, next),
        Kind::Positional => Ok(positional(store, arg)),
    }
}

fn positional(store: &mut Store, word: &str) -> usize {
    store.positional(word);
    1
}

fn classify(arg: &str) -> Result<Kind<'_>, JsError> {
    let Some(rest) = arg.strip_prefix("--") else {
        return Ok(classify_single_dash(arg));
    };
    if has_equals_after_dot(rest) {
        return split_assign(rest);
    }
    match rest
        .strip_prefix("no-")
        .map(dots)
        .filter(|key| !key.is_empty())
    {
        Some(key) => Ok(Kind::Negated(key)),
        None => Ok(long_or_word(rest)),
    }
}

fn classify_single_dash(arg: &str) -> Kind<'_> {
    let mut chars = arg.chars();
    match (chars.next(), chars.next()) {
        (Some('-'), Some(second)) if second != '-' => Kind::Short,
        _ => Kind::Positional,
    }
}

fn long_or_word(rest: &str) -> Kind<'_> {
    match dots(rest) {
        "" => Kind::Positional,
        key => Kind::Long(key),
    }
}

/// `^--.+=`: the dot does not match a line break, `=` is not the first character.
fn has_equals_after_dot(rest: &str) -> bool {
    dots(rest).bytes().skip(1).any(|b| b == b'=')
}

/// `^--([^=]+)=([\s\S]*)$`; for `--==x` there is no match, and `m[1]` crashes in JS.
fn split_assign(rest: &str) -> Result<Kind<'_>, JsError> {
    match rest.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok(Kind::Assign(key, value)),
        _ => Err(JsError::Type(
            "Cannot read properties of null (reading '1')".to_owned(),
        )),
    }
}

/// Start of the string up to the first line break: what `.` takes in a regular expression.
fn dots(s: &str) -> &str {
    let end = s
        .find(['\n', '\r', '\u{2028}', '\u{2029}'])
        .unwrap_or(s.len());
    &s[..end]
}

/// `/^(-|--)[^-]/`: the next token itself looks like a flag.
fn looks_like_flag(word: &str) -> bool {
    let after_dashes = word.trim_start_matches('-');
    let dashes = word.len() - after_dashes.len();
    (1..=2).contains(&dashes) && !after_dashes.is_empty()
}

/// `--key [value]`: the value is the next token if it does not look like a flag.
fn long_flag(store: &mut Store, key: &str, next: Option<&str>) -> Result<usize, JsError> {
    take_value(store, key, next.filter(|w| !looks_like_flag(w)))
}

fn take_value(store: &mut Store, key: &str, value: Option<&str>) -> Result<usize, JsError> {
    match value {
        Some(word) => store.set_text(key, word).map(|()| 2),
        None => store.set_bool(key, true).map(|()| 1),
    }
}

/// `-abc [value]`: cluster letters, the last one may take the next token.
fn short_flag(store: &mut Store, arg: &str, next: Option<&str>) -> Result<usize, JsError> {
    let chars: Vec<char> = arg.chars().collect();
    let (last, letters) = chars[1..].split_last().expect("`-x…` is two chars or more");
    if cluster(store, &chars, letters.len())? || *last == '-' {
        return Ok(1);
    }
    let value = next.filter(|w| !w.is_empty() && !looks_like_flag(w));
    take_value(store, &last.to_string(), value)
}

/// Cluster letters one by one; `true` — parsing stopped at a value inside the token.
fn cluster(store: &mut Store, chars: &[char], count: usize) -> Result<bool, JsError> {
    for j in 0..count {
        let rest: String = chars[j + 2..].iter().collect();
        let following = (j + 1 < count).then(|| chars[j + 2]);
        if letter(store, chars[j + 1], following, &rest)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One cluster letter; `true` — the value was taken from the rest of the token.
fn letter(
    store: &mut Store,
    letter: char,
    following: Option<char>,
    rest: &str,
) -> Result<bool, JsError> {
    let key = letter.to_string();
    if rest == "-" {
        return store.set_text(&key, rest).map(|()| false);
    }
    let alpha = letter.is_ascii_alphabetic();
    if let Some(value) = rest.strip_prefix('=').filter(|_| alpha) {
        return store.set_text(&key, value).map(|()| true);
    }
    if (alpha && ends_with_number(rest)) || following.is_some_and(is_non_word) {
        return store.set_text(&key, rest).map(|()| true);
    }
    store.set_bool(&key, true).map(|()| false)
}

/// `/-?\d+(\.\d*)?(e-?\d+)?$/` not anchored at the start: the remainder ends with a digit or `digit.`.
fn ends_with_number(rest: &str) -> bool {
    let mut back = rest.chars().rev();
    match (back.next(), back.next()) {
        (Some(last), _) if last.is_ascii_digit() => true,
        (Some('.'), Some(before)) => before.is_ascii_digit(),
        _ => false,
    }
}

/// `\W`: not `[A-Za-z0-9_]`.
fn is_non_word(c: char) -> bool {
    !(c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "argv_tests.rs"]
mod tests;
