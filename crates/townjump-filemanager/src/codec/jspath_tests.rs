use super::*;

#[test]
fn join_matches_node() {
    let cases: [(&[&str], &str); 9] = [
        (&[".", "/"], "./"),
        (&["/tmp/r", "/"], "/tmp/r/"),
        (&[".", "/../x"], "../x"),
        (&["/tmp/r", "/../../.."], "/"),
        (&["", ""], "."),
        (&["a//b", "./c/"], "a/b/c/"),
        (&["//", "x"], "/x"),
        (&["root", "http://x/"], "root/http:/x/"),
        (&["x/..", ".."], ".."),
    ];
    for (parts, want) in cases {
        assert_eq!(join(parts), want, "{parts:?}");
    }
}

#[test]
fn ext_and_base() {
    assert_eq!(extname("f.tar.gz"), ".gz");
    assert_eq!(extname(".html"), "");
    assert_eq!(extname("a."), ".");
    assert_eq!(extname(".."), "");
    assert_eq!(extname("a.b/"), ".b");
    assert_eq!(basename("a.b/"), "a.b");
}

#[test]
fn escapes_root_cases() {
    let cases = [
        ("/tmp/r", "/tmp/r/x", false),
        ("..", "../../x", true),
        ("../..", "../../../x", true),
        ("..", "../x", false),
        ("../share", "../share/x", false),
        ("/tmp/r", "/tmp/r", false),
        ("/tmp/r", "/tmp/r/", false),
        ("/tmp/r", "/tmp", true),
        ("/tmp/r", "/tmp/rx", true),
        (".", "./", false),
        (".", "a", false),
        (".", "..", true),
        (".", "../b", true),
        ("/", "/", false),
        ("/", "/a/b", false),
        ("./r", "r/x", false),
        ("/tmp/r/", "/tmp/r", false),
    ];
    for (root, full, want) in cases {
        assert_eq!(escapes_root(root, full), want, "{root} {full}");
    }
}

#[test]
fn escapes_root_as_windows_rules() {
    let cases = [
        ("C:/share", "C:/share/..\\..\\Windows\\win.ini", true, true),
        (
            "C:/share",
            "C:/share/..\\..\\Windows\\win.ini",
            false,
            false,
        ),
        ("C:/share", "C:/share/a\\b.txt", true, false),
        ("C:/share", "C:/share/a:s", true, true),
        ("C:\\share", "C:\\share\\x", true, false),
    ];
    for (root, full, win, want) in cases {
        assert_eq!(
            escapes_root_as(root, full, win),
            want,
            "{root} {full} {win}"
        );
    }
}

#[test]
fn win_names_as_rules() {
    let cases = [
        ("C:/share", "C:/share/CON", true, true),
        ("C:/share", "C:/share/CON", false, false),
        ("C:/share", "C:/share/sub/nul.txt", true, true),
        ("C:/share", "C:/share/sub\\NUL", true, true),
        ("C:/share", "C:/share/secret.txt.", true, true),
        ("C:/share", "C:/share/ok.txt", true, false),
        ("C:/share/CON", "C:/share/CON", true, false),
        ("C:/share/CON", "C:/share/CON/x", true, false),
        ("C:/other", "C:/share/CON", true, false),
        (".", "CON", true, true),
        (".", "a/b.", true, true),
        (".", "ok", true, false),
    ];
    for (root, full, win, want) in cases {
        assert_eq!(win_names_as(root, full, win), want, "{root} {full} {win}");
    }
}
