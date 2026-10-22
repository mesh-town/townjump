// Port extension (absent in JS): Windows reserved names for `--safe-mode`.
/// A path component that Win32 maps to a device or trims to another name.
pub(crate) fn unsafe_component(c: &str) -> bool {
    if c.ends_with(['.', ' ']) {
        return true;
    }
    if CONSOLE.iter().any(|n| c.eq_ignore_ascii_case(n)) {
        return true;
    }
    let base = c.split('.').next().unwrap_or("").trim_end_matches(' ');
    is_device(&base.to_uppercase())
}

/// Opens the console when the name stays bare (relative root `.`).
const CONSOLE: [&str; 2] = ["CONIN$", "CONOUT$"];
const PLAIN: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
/// `COM`/`LPT` followed by one digit, or by a superscript 1, 2, 3.
const NUMBERED: [&str; 2] = ["COM", "LPT"];
const SUPERSCRIPTS: [char; 3] = ['\u{b9}', '\u{b2}', '\u{b3}'];

/// `base` is already upper-cased.
fn is_device(base: &str) -> bool {
    if PLAIN.contains(&base) {
        return true;
    }
    let Some(prefix) = NUMBERED.iter().find_map(|p| base.strip_prefix(p)) else {
        return false;
    };
    let mut rest = prefix.chars();
    match (rest.next(), rest.next()) {
        (Some(d), None) => d.is_ascii_digit() || SUPERSCRIPTS.contains(&d),
        _ => false,
    }
}

#[cfg(test)]
#[path = "winname_tests.rs"]
mod tests;
