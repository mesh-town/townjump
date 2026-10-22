// JS: livefiles/index.js:listDirectory (page HTML, form, table row, icons; resources moved to page/*).
// Source livefiles 1.1.0, GPLv3, compatible with AGPL-3.0-only via GPLv3 §13. Resources are copied verbatim from the livefiles source.
pub(crate) const FOLDER_ICON: &str = include_str!("icon-folder.svg");
pub(crate) const FILE_ICON: &str = include_str!("icon-file.svg");
const PAGE_HTML: &str = include_str!("page.html");
const FORM_HTML: &str = include_str!("form.html");
const LOGO: &str = include_str!("logo.txt");

pub(crate) fn page(title_escaped: &str, rows: &str, form: &str) -> String {
    fill(
        PAGE_HTML,
        &[
            ("LOGO", LOGO),
            ("TITLE_PATH", title_escaped),
            ("ROWS", rows),
            ("FORM", form),
        ],
    )
}

pub(crate) fn form(action: &str, options: &str) -> String {
    fill(FORM_HTML, &[("ACTION", action), ("OPTIONS", options)])
}

pub(crate) fn row(icon: &str, href: &str, name_escaped: &str, button: &str, size: &str) -> String {
    format!(
        "<tr><td class=\"file--name\">{icon}<a href=\"{href}\">{name_escaped}</a></td><td class=\"download--btn\">{button}</td><td>{size}</td></tr>"
    )
}

/// One pass over the template: inserted values are not parsed again.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match label_at(rest, values) {
            Some((value, len)) => {
                out.push_str(value);
                rest = &rest[len..];
            }
            None => {
                out.push_str("{{");
                rest = &rest[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn label_at<'a>(rest: &str, values: &[(&str, &'a str)]) -> Option<(&'a str, usize)> {
    for &(name, value) in values {
        let token = format!("{{{{{name}}}}}");
        if rest.starts_with(&token) {
            return Some((value, token.len()));
        }
    }
    None
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
