// JS: ECMAScript `StringToNumber` (unary `+` on a string) and `String.prototype.trim`.
// Related: port.rs (the only consumer).

/// JS whitespace: Unicode `White_Space` without U+0085, plus U+FEFF.
pub fn trim_js(s: &str) -> &str {
    s.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// `+s`: `NaN` if the string is not a JS numeric literal.
pub fn string_to_number(s: &str) -> f64 {
    let t = trim_js(s);
    match t {
        "" => 0.0,
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => radix_literal(t).unwrap_or_else(|| decimal_literal(t)),
    }
}

/// `0x…`, `0o…`, `0b…` (unsigned); `None` means no prefix.
fn radix_literal(t: &str) -> Option<f64> {
    let (radix, digits) = match t.get(..2)? {
        "0x" | "0X" => (16, &t[2..]),
        "0o" | "0O" => (8, &t[2..]),
        "0b" | "0B" => (2, &t[2..]),
        _ => return None,
    };
    Some(fold_digits(digits, radix))
}

fn fold_digits(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let step =
        |acc: Option<f64>, c: char| Some(acc? * f64::from(radix) + f64::from(c.to_digit(radix)?));
    digits
        .chars()
        .try_fold(0.0, |a, c| step(Some(a), c))
        .unwrap_or(f64::NAN)
}

/// `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`
fn decimal_literal(t: &str) -> f64 {
    let unsigned = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    if mantissa_ok(mantissa) && exponent_ok(exponent) {
        return t.parse().unwrap_or(f64::NAN);
    }
    f64::NAN
}

fn all_digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

fn mantissa_ok(m: &str) -> bool {
    let (int, frac) = m.split_once('.').unwrap_or((m, ""));
    all_digits(int) && all_digits(frac) && !(int.is_empty() && frac.is_empty())
}

fn exponent_ok(e: &str) -> bool {
    let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
    !digits.is_empty() && all_digits(digits)
}
