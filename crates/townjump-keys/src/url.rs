// JS: holesail/src/index.js:Holesail.urlParser (`hs://…` string, the secure flag is the 6th character `s`).
// Related: server_keys.rs and client_identity.rs take the key and mode from here. Input is already `String(url || '')`.

const PROTOCOL: &str = "hs://";

/// `urlParser` result: the key and the "private" flag (`None` means not set, parsing never gives `false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub key: String,
    pub secure: Option<bool>,
}

/// `Holesail.urlParser`; indices are in UTF-16 code units, like `String.substring`.
pub fn parse_url(url: &str) -> ParsedUrl {
    let units: Vec<u16> = url.encode_utf16().collect();
    let secure = (units.get(5) == Some(&u16::from(b's'))).then_some(true);
    let has_prefix =
        units.starts_with(&PROTOCOL.encode_utf16().collect::<Vec<_>>()) && units.len() >= 9;
    let key = if has_prefix {
        String::from_utf16_lossy(&units[9..])
    } else {
        url.to_string()
    };
    ParsedUrl { key, secure }
}

/// `'hs://' + ('s000' | '0000') + key`.
pub fn format_url(key: &str, secure: bool) -> String {
    let mark = if secure { "s000" } else { "0000" };
    format!("{PROTOCOL}{mark}{key}")
}

#[cfg(test)]
#[path = "url_tests.rs"]
mod tests;
