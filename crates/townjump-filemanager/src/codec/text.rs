// JS: livefiles/index.js:formatBytes/escapeHtml, Number#toFixed, parseInt(s, 10), String#replace, Number#toString.
const SIZES: [&str; 9] = ["Bytes", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

pub(crate) fn format_bytes(b: f64) -> String {
    if b == 0.0 {
        return "0 Bytes".to_string();
    }
    let i = ((b.ln() / 1024f64.ln()).floor().max(0.0) as usize).min(SIZES.len() - 1);
    format!(
        "{} {}",
        to_fixed2_trimmed(b / 1024f64.powi(i as i32)),
        SIZES[i]
    )
}

/// `parseFloat(x.toFixed(2))` as a string; a tie (x = k/8, k odd) rounds up, as in `toFixed`.
fn to_fixed2_trimmed(x: f64) -> String {
    let s = if is_tie(x) {
        tie_up(x)
    } else {
        format!("{x:.2}")
    };
    if !s.contains('.') {
        return s;
    }
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn is_tie(x: f64) -> bool {
    let k = x * 8.0;
    k.fract() == 0.0 && k % 2.0 != 0.0
}

fn tie_up(x: f64) -> String {
    let n = (x * 100.0).ceil() as u64;
    format!("{}.{:02}", n / 100, n % 100)
}

pub(crate) fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            _ => out.push(c),
        }
    }
    out
}

/// JS whitespace (WhiteSpace + LineTerminator) that `parseInt` strips from the left.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// NaN — no number.
pub(crate) fn parse_int10(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_space);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return f64::NAN;
    }
    let v: f64 = t[..digits].parse().unwrap_or(f64::NAN);
    if neg { -v } else { v }
}

/// Constant-time comparison, independent of where the first difference is; different lengths -> `false`.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn replace_first(s: &str, pat: &str, with: &str) -> String {
    s.replacen(pat, with, 1)
}

pub(crate) fn js_num(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        let word = if n > 0.0 { "Infinity" } else { "-Infinity" };
        return word.to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    let a = n.abs();
    if (1e-6..1e21).contains(&a) {
        return format!("{n}");
    }
    exp_form(n)
}

fn exp_form(n: f64) -> String {
    let s = format!("{n:e}");
    let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let sign = if exp.starts_with('-') { "" } else { "+" };
    format!("{mant}e{sign}{exp}")
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
