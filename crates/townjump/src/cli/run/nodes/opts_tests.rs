use super::*;
use crate::cli::argv::parse_argv;

fn opts(args: &[&str]) -> Result<Option<NodesOpts>, String> {
    let argv = args.iter().map(|s| (*s).to_owned()).collect();
    nodes_opts(&parse_argv(argv).unwrap())
}

#[test]
fn absent_flag_is_none() {
    assert!(opts(&["--live", "1"]).unwrap().is_none());
}

#[test]
fn path_with_defaults() {
    let got = opts(&["--live", "1", "--save-nodes", "n.txt"])
        .unwrap()
        .unwrap();
    assert_eq!(got.path, PathBuf::from("n.txt"));
    assert_eq!(got.ttl, Duration::from_secs(168 * 3600));
    assert_eq!(got.every, Duration::from_secs(300));
}

#[test]
fn numeric_path_accepted() {
    let got = opts(&["--save-nodes", "123"]).unwrap().unwrap();
    assert_eq!(got.path, PathBuf::from("123"));
}

#[test]
fn bare_flag_is_error() {
    assert_eq!(
        opts(&["--save-nodes"]).err(),
        Some("--save-nodes needs a file path".to_owned())
    );
}

#[test]
fn ttl_and_every_parsed() {
    let got = opts(&[
        "--save-nodes",
        "n",
        "--save-nodes-ttl",
        "24",
        "--save-nodes-every",
        "5",
    ]);
    let got = got.unwrap().unwrap();
    assert_eq!(got.ttl, Duration::from_secs(24 * 3600));
    assert_eq!(got.every, Duration::from_secs(5));
}

#[test]
fn bad_numbers_are_errors() {
    let ttl = "--save-nodes-ttl needs a positive number of hours";
    let every = "--save-nodes-every needs a positive number of seconds";
    for bad in ["0", "-1", "abc"] {
        assert_eq!(
            opts(&["--save-nodes", "n", "--save-nodes-ttl", bad])
                .err()
                .as_deref(),
            Some(ttl),
            "{bad}"
        );
        assert_eq!(
            opts(&["--save-nodes", "n", "--save-nodes-every", bad])
                .err()
                .as_deref(),
            Some(every),
            "{bad}"
        );
    }
    assert_eq!(
        opts(&["--save-nodes", "n", "--save-nodes-ttl"])
            .err()
            .as_deref(),
        Some(ttl)
    );
    assert_eq!(
        opts(&["--save-nodes", "n", "--save-nodes-every"])
            .err()
            .as_deref(),
        Some(every)
    );
}

#[test]
fn extras_without_main_flag_are_ignored() {
    assert!(opts(&["--save-nodes-ttl", "5"]).unwrap().is_none());
}
