// JS: ECMAScript `Number.prototype.toString` (Number::toString, 6.1.6.1.20).
// Related: value.rs (`String(v)` calls this for numbers), json.rs (numbers in JSON.stringify).

/// Decimal form of a number as in JS: `1e21`, `1.2345678901234568e+38`, `-0` -> `0`, `NaN`, `Infinity`.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n < 0.0 {
        return format!("-{}", number_to_string(-n));
    }
    if n.is_infinite() {
        return "Infinity".into();
    }
    finite_positive(n)
}

/// Rust `{:e}` prints the shortest digits that round-trip to the number, the same ones ECMAScript takes.
fn finite_positive(n: f64) -> String {
    let text = format!("{n:e}");
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let point = exp.parse::<i32>().unwrap_or(0) + 1;
    layout(&digits, point)
}

/// Steps 6-12 of Number::toString: `digits` are the significant digits, `point` is the decimal point position relative to them.
fn layout(digits: &str, point: i32) -> String {
    let k = digits.len() as i32;
    let zeros = |count: i32| "0".repeat(count.max(0) as usize);
    match point {
        p if k <= p && p <= 21 => format!("{digits}{}", zeros(p - k)),
        p if 0 < p && p <= 21 => format!("{}.{}", &digits[..p as usize], &digits[p as usize..]),
        p if -6 < p && p <= 0 => format!("0.{}{digits}", zeros(-p)),
        p => exponent_form(digits, p - 1),
    }
}

fn exponent_form(digits: &str, e: i32) -> String {
    let sign = if e < 0 { '-' } else { '+' };
    let (head, tail) = digits.split_at(1);
    let fraction = if tail.is_empty() {
        String::new()
    } else {
        format!(".{tail}")
    };
    format!("{head}{fraction}e{sign}{}", e.abs())
}

#[cfg(test)]
#[path = "number_tests.rs"]
mod tests;
