// JS: ECMAScript decodeURIComponent (strict UTF-8; any error is URIError: URI malformed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UriMalformed;

pub(crate) fn decode_uri_component(s: &str) -> Result<String, UriMalformed> {
    if !s.contains('%') {
        return Ok(s.to_owned());
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let first = hex_byte(b, i)?;
        i += 3;
        out.push(first);
        let extra = match first {
            0x00..=0x7F => 0,
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF7 => 3,
            _ => return Err(UriMalformed),
        };
        for _ in 0..extra {
            let c = hex_byte(b, i)?;
            if c & 0xC0 != 0x80 {
                return Err(UriMalformed);
            }
            out.push(c);
            i += 3;
        }
    }
    String::from_utf8(out).map_err(|_| UriMalformed)
}

/// Byte from `%XX` at position `i` (a `%` must be there).
fn hex_byte(b: &[u8], i: usize) -> Result<u8, UriMalformed> {
    if i + 2 >= b.len() {
        return Err(UriMalformed);
    }
    if b[i] != b'%' {
        return Err(UriMalformed);
    }
    let h = hex_val(b[i + 1]).ok_or(UriMalformed)?;
    let l = hex_val(b[i + 2]).ok_or(UriMalformed)?;
    Ok(h << 4 | l)
}

pub(crate) fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "uri_tests.rs"]
mod tests;
