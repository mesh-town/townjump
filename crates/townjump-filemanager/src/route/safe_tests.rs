use super::*;

#[test]
fn host_allowed_cases() {
    for h in [
        "127.0.0.1:5409",
        "[::1]:80",
        "[::1]",
        "LOCALHOST",
        "localhost",
        "app.localhost:5409",
        "Host.Local:8989",
        "nas",
        "NAS:80",
    ] {
        assert!(host_allowed(h), "{h}");
    }
    for h in [
        "evil.example",
        "evil.example:5409",
        "127.0.0.1.nip.io",
        "localhost.evil.example",
        "host.local.",
        "localhost:abc",
        "localhost:",
        "::1",
        "",
        "[::1",
    ] {
        assert!(!host_allowed(h), "{h}");
    }
}

#[test]
fn same_origin_cases() {
    assert!(same_origin("http://localhost:5409", Some("localhost:5409")));
    assert!(same_origin("http://LOCALHOST:5409", Some("localhost:5409")));
    assert!(!same_origin("http://evil.example", Some("localhost")));
    assert!(!same_origin("null", Some("localhost")));
    assert!(!same_origin("http://localhost", None));
}

#[test]
fn disposition_cases() {
    assert_eq!(
        disposition("a\"; filename*=UTF-8''evil.html"),
        "attachment; filename=\"a__ filename*=UTF-8''evil.html\"; filename*=UTF-8''a%22%3B%20filename%2A%3DUTF-8%27%27evil.html"
    );
    assert_eq!(
        disposition("φαηλ.txt"),
        "attachment; filename=\"____.txt\"; filename*=UTF-8''%CF%86%CE%B1%CE%B7%CE%BB.txt"
    );
    let d = disposition("a\\b");
    assert!(
        d.contains("filename=\"a_b\"") && d.ends_with("a%5Cb"),
        "{d}"
    );
}

#[test]
fn location_cases() {
    assert_eq!(location("/παπκα/"), "/%CF%80%CE%B1%CF%80%CE%BA%CE%B1/");
    assert_eq!(location("/a b/"), "/a%20b/");
    assert_eq!(location("/100%/"), "/100%25/");
    assert_eq!(location("/a?b=1#c"), "/a%3Fb=1%23c");
    assert_eq!(location("/a#b/"), "/a%23b/");
}

#[test]
fn html_cases() {
    assert_eq!(html(false, "\"><"), "\"><");
    assert_eq!(html(true, "\"><"), "&quot;&gt;&lt;");
}

#[cfg(unix)]
mod fs {
    use super::super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    struct Dirs(PathBuf);
    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(tag: &str) -> (Dirs, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("hs-safe-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let out = base.join("out");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        std::fs::write(out.join("s.txt"), "S").unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        (Dirs(base), root, out)
    }

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn confined_cases() {
        let (_g, root, out) = setup("confined");
        symlink(root.join("sub"), root.join("in")).unwrap();
        symlink(out.join("s.txt"), root.join("peek")).unwrap();
        symlink(&out, root.join("outdir")).unwrap();
        symlink(out.join("none"), root.join("dang")).unwrap();
        assert!(confined(&root, &s(&root.join("f.txt"))).await);
        assert!(confined(&root, &s(&root.join("new/deeper"))).await);
        assert!(confined(&root, &s(&root.join("in"))).await);
        assert!(!confined(&root, &s(&root.join("peek"))).await);
        assert!(!confined(&root, &s(&root.join("outdir/more/deeper"))).await);
        assert!(confined(&root, &s(&root.join("dang"))).await);
        assert!(confined(&root, &s(&root.join("nope"))).await);
    }

    #[tokio::test]
    async fn confined_refuses_on_non_not_found_errors() {
        use std::os::unix::fs::PermissionsExt;
        let (_g, root, _out) = setup("denied");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        let mode = |m| std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(m));
        mode(0).unwrap();
        let skip = std::fs::read_dir(&locked).is_ok(); // root bypasses permissions
        let got = confined(&root, &s(&locked.join("x/y"))).await;
        mode(0o755).unwrap();
        assert!(skip || !got);
    }

    #[tokio::test]
    async fn confined_root_symlink_path() {
        let (_g, root, _out) = setup("rootlink");
        let link = root.parent().unwrap().join("rl");
        symlink(&root, &link).unwrap();
        assert!(confined(&root, &s(&link.join("f.txt"))).await);
        assert!(!confined(&root, &s(&link.join("../out/s.txt"))).await);
    }

    #[tokio::test]
    async fn create_new_cases() {
        let (_g, root, out) = setup("create");
        let fresh = s(&root.join("fresh.txt"));
        create_new(&fresh).await.unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "");
        let old = s(&root.join("f.txt"));
        let e = create_new(&old).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "x");
        symlink(out.join("new.txt"), root.join("dang")).unwrap();
        let e = create_new(&s(&root.join("dang"))).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(!out.join("new.txt").exists());
    }
}
