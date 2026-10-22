use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

const HOUR: Duration = Duration::from_secs(3600);

fn addr(text: &str) -> SocketAddrV4 {
    text.parse().unwrap()
}

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("townjump-nodes-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

mod parse {
    use super::*;

    #[test]
    fn round_trip() {
        let nodes = vec![addr("1.2.3.4:5"), addr("5.6.7.8:9000")];
        assert_eq!(parse(&render(&nodes, 100), 100, HOUR), nodes);
    }

    #[test]
    fn wrong_header_is_empty() {
        assert!(parse("townjump-nodes 2\nsaved 100\n1.2.3.4:5\n", 100, HOUR).is_empty());
        assert!(parse("garbage\nsaved 100\n1.2.3.4:5\n", 100, HOUR).is_empty());
        assert!(parse("", 100, HOUR).is_empty());
    }

    #[test]
    fn missing_saved_is_empty() {
        assert!(parse("townjump-nodes 1\n1.2.3.4:5\n", 100, HOUR).is_empty());
        assert!(parse("townjump-nodes 1\nsaved x\n1.2.3.4:5\n", 100, HOUR).is_empty());
    }

    #[test]
    fn expired_is_empty() {
        let text = render(&[addr("1.2.3.4:5")], 1000);
        assert!(parse(&text, 1000 + 3600 + 1, HOUR).is_empty());
    }

    #[test]
    fn age_equal_ttl_is_kept() {
        let text = render(&[addr("1.2.3.4:5")], 1000);
        assert_eq!(parse(&text, 1000 + 3600, HOUR).len(), 1);
    }

    #[test]
    fn future_saved_is_kept() {
        let text = render(&[addr("1.2.3.4:5")], 5000);
        assert_eq!(parse(&text, 100, HOUR).len(), 1);
    }

    #[test]
    fn bad_lines_are_skipped() {
        let text = "townjump-nodes 1\nsaved 10\nexample.com:80\n[::1]:80\n1.2.3.4:0\n1.2.3.4:70000\n\n1.2.3.4:5\n";
        assert_eq!(parse(text, 10, HOUR), vec![addr("1.2.3.4:5")]);
    }

    #[test]
    fn duplicates_removed_order_kept() {
        let text =
            "townjump-nodes 1\nsaved 10\n2.2.2.2:2\n1.1.1.1:1\n2.2.2.2:2\n3.3.3.3:3\n1.1.1.1:1\n";
        let want = vec![addr("2.2.2.2:2"), addr("1.1.1.1:1"), addr("3.3.3.3:3")];
        assert_eq!(parse(text, 10, HOUR), want);
    }

    #[test]
    fn capped_at_max_nodes() {
        let all: Vec<SocketAddrV4> = (0..300u32)
            .map(|i| addr(&format!("10.0.{}.{}:1000", i / 200, i % 200 + 1)))
            .collect();
        let got = parse(&render(&all, 10), 10, HOUR);
        assert_eq!(got.len(), MAX_NODES);
        assert_eq!(got, all[..MAX_NODES]);
    }
}

mod save {
    use super::*;

    #[test]
    fn writes_render_output() {
        let dir = temp_dir();
        let file = dir.join("nodes.txt");
        let nodes = [addr("1.2.3.4:5")];
        save(&file, &nodes, 42).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), render(&nodes, 42));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replaces_previous_content() {
        let dir = temp_dir();
        let file = dir.join("nodes.txt");
        save(&file, &[addr("1.1.1.1:1")], 1).unwrap();
        save(&file, &[addr("2.2.2.2:2")], 2).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            render(&[addr("2.2.2.2:2")], 2)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn leaves_no_temp_file() {
        let dir = temp_dir();
        save(&dir.join("nodes.txt"), &[addr("1.1.1.1:1")], 1).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_directory_is_error() {
        let dir = temp_dir();
        let sub = dir.join("absent");
        assert!(save(&sub.join("nodes.txt"), &[addr("1.1.1.1:1")], 1).is_err());
        assert!(!sub.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

mod load {
    use super::*;

    #[test]
    fn missing_file_is_empty() {
        let dir = temp_dir();
        assert!(load(&dir.join("none.txt"), 1, HOUR).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn binary_garbage_is_empty() {
        let dir = temp_dir();
        let file = dir.join("bin.txt");
        std::fs::write(&file, [0xff; 64]).unwrap();
        assert!(load(&file, 1, HOUR).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
