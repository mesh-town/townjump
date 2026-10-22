// JS: z32 1.1.0 (`index.js`: encode, decode), alphabet `ybndrfg8ejkmcpqxot1uwisza345h769`.
// Related: error.rs (`KeysError::InvalidCharacter`), url.rs and client_identity.rs (consumers).
use crate::error::KeysError;

const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
/// Bytes for a tail of `r` characters (`r = len % 8`): as in JS, not `ceil(5r/8)`.
const TAIL_BYTES: [usize; 8] = [0, 1, 1, 2, 2, 3, 4, 4];

/// z-base-32 without padding.
pub fn encode(b: &[u8]) -> String {
    let mut out = String::with_capacity((b.len() * 8).div_ceil(5));
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in b {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        drain(&mut out, &mut acc, &mut bits);
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn drain(out: &mut String, acc: &mut u32, bits: &mut u32) {
    while *bits >= 5 {
        *bits -= 5;
        out.push(ALPHABET[((*acc >> *bits) & 31) as usize] as char);
    }
    *acc &= (1 << *bits) - 1;
}

/// Inverse of `encode`; an invalid character gives `KeysError::InvalidCharacter`.
pub fn decode(s: &str) -> Result<Vec<u8>, KeysError> {
    let units = s.encode_utf16().count();
    let want = units / 8 * 5 + TAIL_BYTES[units % 8];
    let mut out = Vec::with_capacity(want);
    let (mut acc, mut bits) = (0u32, 0u32);
    let quintets = utf16_positions(s).map(|(position, ch)| quintet(ch, position));
    // as in JS: the tail is padded with zero quintets up to the needed number of bytes
    for q in quintets.chain(std::iter::repeat_with(|| Ok(0))) {
        if out.len() == want {
            break;
        }
        acc = (acc << 5) | q?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Characters with an index in UTF-16 code units, like `s[i]` in JS.
fn utf16_positions(s: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    s.chars().scan(0, |pos, ch| {
        let at = *pos;
        *pos += ch.len_utf16();
        Some((at, ch))
    })
}

fn quintet(ch: char, position: usize) -> Result<u32, KeysError> {
    let found = ALPHABET.iter().position(|&a| char::from(a) == ch);
    found
        .map(|i| i as u32)
        .ok_or_else(|| KeysError::InvalidCharacter {
            ch: ch.to_string(),
            position,
        })
}

#[cfg(test)]
#[path = "z32_tests.rs"]
mod tests;
