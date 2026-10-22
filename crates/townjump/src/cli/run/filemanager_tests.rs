use super::*;
use crate::cli::argv::parse_argv;

fn safe(args: &[&str]) -> bool {
    let argv = args.iter().map(|s| (*s).to_owned()).collect();
    let parsed = parse_argv(argv).unwrap();
    files_opts(&parsed, Arc::new(|_| {})).safe_mode
}

#[test]
fn safe_mode_flag_is_read_from_argv() {
    assert!(safe(&["--filemanager", ".", "--safe-mode"]));
    assert!(safe(&["--safe-mode", "--filemanager", "."]));
    assert!(!safe(&["--filemanager", ".", "--no-safe-mode"]));
    assert!(!safe(&["--filemanager", "."]));
    for off in ["false", "FALSE", "False", "0", "no", "No"] {
        let arg = format!("--safe-mode={off}");
        assert!(!safe(&["--filemanager", ".", &arg]), "{arg}");
    }
    for on in ["true", "1", "yes", "x"] {
        let arg = format!("--safe-mode={on}");
        assert!(safe(&["--filemanager", ".", &arg]), "{arg}");
    }
}
