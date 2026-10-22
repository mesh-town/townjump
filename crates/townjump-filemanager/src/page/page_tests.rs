use super::*;

#[test]
fn row_matches_js_template() {
    let got = row("<i/>", "/a", "a", "<a>B</a>", "1 KB");
    let want = "<tr><td class=\"file--name\"><i/><a href=\"/a\">a</a></td><td class=\"download--btn\"><a>B</a></td><td>1 KB</td></tr>";
    assert_eq!(got, want);
}

#[test]
fn page_substitutes_all_labels() {
    let html = page("/x", "<tr>ROW</tr>", "<form>F</form>");
    assert!(html.contains("<h1>Folder and Files: /x</h1>"));
    assert!(html.contains("<tr>ROW</tr>"));
    assert!(html.contains("<form>F</form>"));
    assert!(html.contains("src=\"data:image/png;base64,"));
    assert!(!html.contains("{{"));
}

#[test]
fn inserted_values_are_not_reparsed() {
    let html = page("{{ROWS}}", "R", "");
    assert!(html.contains("Folder and Files: {{ROWS}}</h1>"));
}

#[test]
fn form_substitutes_action_and_options() {
    let f = form("/d", "<option>o</option>");
    assert!(f.contains("<form method=\"POST\" action=\"/d\">"));
    assert!(f.contains("<option>o</option>"));
    assert!(!f.contains("{{"));
}

#[test]
fn icons_are_svg() {
    assert!(FOLDER_ICON.starts_with("<svg") && FOLDER_ICON.ends_with("</svg>"));
    assert!(FILE_ICON.starts_with("<svg") && FILE_ICON.ends_with("</svg>"));
}
