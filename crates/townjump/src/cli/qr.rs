// JS: qrcode-terminal 0.12.0 (`qrcode.generate(url, { small: true })`), error correction L, half blocks.
// See also: banner.rs; checked against testvectors/holesail-cli/golden/ (the `qrcode` crate first, the port where they differ).

mod encode;
mod lost;
mod matrix;
mod tables;

/// QR of the connection string in half blocks `▀▄█ ` with an empty line at the end, like `console.log`.
pub fn qr_small(url: &str) -> String {
    let version = encode::pick_version(url);
    let code = matrix::build(version, &encode::codewords(url, version));
    let width = code.size;
    let dark = |x: usize, y: usize| y < width && code.dark(y, x);
    let mut out = "▄".repeat(width + 2) + "\n";
    for y in (0..width).step_by(2) {
        out.push('█');
        out.extend((0..width).map(|x| cell(dark(x, y), dark(x, y + 1))));
        out.push_str("█\n");
    }
    out.push('\n');
    out
}

/// Half-block cell: upper and lower module (a dark module is empty space).
fn cell(top: bool, bottom: bool) -> char {
    match (top, bottom) {
        (false, false) => '█',
        (false, true) => '▀',
        (true, false) => '▄',
        (true, true) => ' ',
    }
}

#[cfg(test)]
#[path = "qr_tests.rs"]
mod tests;
