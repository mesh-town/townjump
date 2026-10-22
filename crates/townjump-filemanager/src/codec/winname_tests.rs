use super::*;

#[test]
fn bad_names_are_unsafe() {
    let bad = [
        "CON",
        "con",
        "Nul",
        "nul.txt",
        "NUL.tar.gz",
        "aux.",
        "COM1",
        "com9.log",
        "LPT1",
        "lpt9.x",
        "COM\u{b9}",
        "LPT\u{b3}.txt",
        "CONIN$",
        "conout$",
        "COM0",
        "lpt0",
        "secret.txt.",
        "secret.txt ",
        " ",
        "a. .",
        "nul .txt",
        "PRN",
        "com\u{b2}",
    ];
    for c in bad {
        assert!(unsafe_component(c), "{c:?}");
    }
}

#[test]
fn good_names_are_safe() {
    let good = [
        "CONSOLE",
        "console.txt",
        "com10",
        "COM",
        "LPT",
        "lpt10.txt",
        "nulx",
        "xnul",
        ".nul",
        "comma.txt",
        "a.b",
        "secret.txt",
        ".hidden",
        "a..b",
        "\u{444}\u{430}\u{439}\u{43b}.txt",
        "ok",
        "CONIN$x",
    ];
    for c in good {
        assert!(!unsafe_component(c), "{c:?}");
    }
}
