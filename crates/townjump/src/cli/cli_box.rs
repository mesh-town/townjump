// JS: cli-box 6.0.10 (banner frame) and barely-colours 1.0.0; format — cli.md, section 6.
// See also: banner.rs assembles the content and calls the frame from here.
use std::collections::VecDeque;

const MIN_WIDTH: usize = 100;
const MIN_HEIGHT: usize = 10;

/// A frame text line: visible characters, padding, escape codes with positions in the visible text.
struct Line {
    text: Vec<char>,
    x: usize,
    y: usize,
    codes: VecDeque<(usize, String)>,
}

/// `cliBox('100x10', { text, autoEOL: true, vAlign: 'middle', hAlign: 'middle', stretch: true })`.
pub fn cli_box(text: &str) -> String {
    let parts: Vec<&str> = text.split('\n').map(js_trim).collect();
    let width = box_width(&parts);
    let height = parts.len().max(MIN_HEIGHT);
    let top = (height - parts.len()) / 2;
    let lines = parts
        .iter()
        .enumerate()
        .map(|(i, part)| place(part, width, top + i))
        .collect();
    render(lines, width, height)
}

/// `String.prototype.trim`: ECMAScript whitespace and line terminators (BOM included, U+0085 not).
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c == '\u{feff}' || (c != '\u{85}' && c.is_whitespace()))
}

/// `stretch`: 100 columns if the longest visible line is no wider than 98, otherwise length + 2.
fn box_width(parts: &[&str]) -> usize {
    let longest = parts
        .iter()
        .map(|p| split_codes(p).0.len())
        .max()
        .unwrap_or(0);
    MIN_WIDTH.max(longest + 2)
}

/// Separates `\e[…m` from the visible text; the code position is an index in the visible text.
fn split_codes(line: &str) -> (Vec<char>, VecDeque<(usize, String)>) {
    let (mut text, mut codes) = (Vec::new(), VecDeque::new());
    let mut rest = line.chars().peekable();
    while let Some(c) = rest.next() {
        if c != '\u{1b}' {
            text.push(c);
            continue;
        }
        let mut code = String::from(c);
        for n in rest.by_ref() {
            code.push(n);
            if n == 'm' {
                break;
            }
        }
        codes.push_back((text.len(), code));
    }
    (text, codes)
}

fn place(part: &str, width: usize, y: usize) -> Line {
    let (text, codes) = split_codes(part);
    let x = (width - 2 - text.len()) / 2;
    Line { text, x, y, codes }
}

fn render(lines: Vec<Line>, width: usize, height: usize) -> String {
    let mut lines: VecDeque<Line> = lines.into();
    let mut next = lines.pop_front();
    let mut last = String::new();
    let mut out = format!("┌{}┐", "─".repeat(width - 2));
    for row in 0..height {
        while next.as_ref().is_some_and(|l| row > l.y) && !lines.is_empty() {
            next = lines.pop_front();
        }
        out.push_str(&format!("\n|{last}"));
        if let Some(line) = next.as_mut() {
            row_cells(line, row, width - 2, &mut last, &mut out);
        } else {
            out.push_str(&" ".repeat(width - 2));
        }
        out.push_str("\u{1b}[0m│");
    }
    out.push_str(&format!("\n└{}┘", "─".repeat(width - 2)));
    out
}

/// Frame line cells; after them the line codes not shown (including those left over with empty text).
fn row_cells(line: &mut Line, row: usize, cols: usize, last: &mut String, out: &mut String) {
    for col in 0..cols {
        let inside = row == line.y && col >= line.x && col < line.x + line.text.len();
        if !inside {
            out.push(' ');
            continue;
        }
        while line.codes.front().is_some_and(|(i, _)| *i == col - line.x) {
            *last = line.codes.pop_front().map(|(_, c)| c).unwrap_or_default();
            out.push_str(last);
        }
        out.push(line.text[col - line.x]);
    }
    while row == line.y && !line.codes.is_empty() {
        *last = line.codes.pop_front().map(|(_, c)| c).unwrap_or_default();
        out.push_str(last);
    }
}
