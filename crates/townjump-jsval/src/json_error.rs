// JS: V8 `JsonParser::ReportUnexpectedToken` and the `JSON.parse` message templates.
// Related: json_parse.rs (calls into here), error.rs. Positions and the context window are in UTF-16 code units.
use crate::error::JsError;

/// Length of the context around the offending character on each side.
const CONTEXT: usize = 10;

/// Parser source text (UTF-16) and position.
pub struct At<'a> {
    pub src: &'a [u16],
    pub pos: usize,
}

fn syntax(text: String) -> JsError {
    JsError::Syntax(text)
}

/// Message of the form "<what> in JSON at position N (line L column C)".
pub fn positioned(at: &At, what: &str) -> JsError {
    let (line, column) = line_column(at.src, at.pos);
    let scope = if what.ends_with("after JSON") {
        ""
    } else {
        " in JSON"
    };
    syntax(format!(
        "{what}{scope} at position {} (line {line} column {column})",
        at.pos
    ))
}

fn line_column(src: &[u16], pos: usize) -> (usize, usize) {
    let (mut line, mut start) = (1, 0);
    for (i, &u) in src.iter().enumerate().take(pos) {
        let crlf_head = u == 13 && src.get(i + 1) == Some(&10);
        if u == 10 || (u == 13 && !crlf_head) {
            line += 1;
            start = i + 1;
        }
    }
    (line, pos - start + 1)
}

/// Unexpected character at position `pos` (or end of input).
pub fn unexpected(at: &At) -> JsError {
    let Some(&unit) = at.src.get(at.pos) else {
        return syntax("Unexpected end of JSON input".into());
    };
    match unit {
        0x30..=0x39 | 0x2d => positioned(at, "Unexpected number"),
        0x22 => positioned(at, "Unexpected string"),
        _ => syntax(token_message(at)),
    }
}

fn token_message(at: &At) -> String {
    let whole = String::from_utf16_lossy(at.src);
    if ["NaN", "Infinity", "undefined"].contains(&whole.as_str()) {
        return format!("\"{whole}\" is not valid JSON");
    }
    let token = String::from_utf16_lossy(&at.src[at.pos..=at.pos]);
    format!(
        "Unexpected token '{token}', {} is not valid JSON",
        excerpt(at)
    )
}

/// Quotes and ellipses around the fragment, as in V8 (a short text is shown whole).
fn excerpt(at: &At) -> String {
    let len = at.src.len();
    if len < CONTEXT * 2 + 1 {
        return format!("\"{}\"", String::from_utf16_lossy(at.src));
    }
    let start = at.pos.saturating_sub(CONTEXT);
    let end = (at.pos + CONTEXT).min(len);
    let piece = String::from_utf16_lossy(&at.src[start..end]);
    let head = if start == 0 { "" } else { "..." };
    let tail = if end == len { "" } else { "..." };
    format!("{head}\"{piece}\"{tail}")
}
