//! Whole server under `--safe-mode`: symlinks pointing outside the root (read, listing, write).
mod support;

use support::{SAFE_AUTH, Setup, TempRoot, start_site};

/// Outside: `secret.txt` (`S`), `outdir/inner.txt` (1000 bytes); the root holds the links and `sub/a.txt`.
/// Returns false (after a note on stderr) when the OS refuses to create links.
fn fixture(root: &TempRoot) -> bool {
    root.write_outside("secret.txt", "S");
    root.write_outside("outdir/inner.txt", &"x".repeat(1000));
    root.write("sub/a.txt", "abc");
    root.write("keep.txt", "data");
    let links = [
        ("peek", root.outside("secret.txt")),
        ("out", root.outside("outdir")),
        ("inner", std::path::PathBuf::from(root.path()).join("sub")),
        ("sub/big", root.outside("outdir/inner.txt")),
    ];
    links.iter().all(|(rel, target)| link(root, rel, target))
}

fn link(root: &TempRoot, rel: &str, target: &std::path::Path) -> bool {
    match root.symlink(rel, target) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipped: symlinks not permitted: {e}");
            false
        }
    }
}

/// `None` means the test is skipped (links cannot be created).
async fn site(safe: bool) -> Option<support::Site> {
    let root = TempRoot::new();
    if !fixture(&root) {
        return None;
    }
    let s = Setup {
        role: Some("admin"),
        username: Some("admin"),
        password: Some("s3cret"),
        files: &[],
        safe_mode: safe,
    };
    Some(start_site(s, root).await)
}

#[tokio::test]
async fn reading_outside_is_forbidden() {
    let Some(s) = site(true).await else { return };
    for t in ["/peek", "/out/", "/out/inner.txt"] {
        let r = s.get_with(t, &[SAFE_AUTH]).await;
        assert_eq!((r.status, r.text().as_str()), (403, "Forbidden"), "{t}");
    }
    assert_eq!(s.get_with("/inner/", &[SAFE_AUTH]).await.status, 200);
}

#[tokio::test]
async fn plain_mode_follows_links() {
    let Some(s) = site(false).await else { return };
    let r = s.get_with("/peek", &[SAFE_AUTH]).await;
    assert_eq!((r.status, r.text().as_str()), (200, "S"));
}

#[tokio::test]
async fn listing_hides_outside_links() {
    let Some(s) = site(true).await else { return };
    let body = s.get_with("/", &[SAFE_AUTH]).await.text();
    assert!(!body.contains("href=\"/peek\"") && !body.contains("href=\"/out\""));
    assert!(body.contains("href=\"/inner\""));
    assert!(body.contains("<td>3 Bytes</td>"), "{body}");
    assert!(!body.contains("1003 Bytes"), "{body}");
    let Some(plain) = site(false).await else {
        return;
    };
    let body = plain.get_with("/", &[SAFE_AUTH]).await.text();
    assert!(body.contains("<td>1003 Bytes</td>"), "{body}");
}

#[tokio::test]
async fn writing_through_links() {
    let Some(s) = site(true).await else { return };
    let r = s
        .post_with("/", &[SAFE_AUTH], "item_type=folder&name=x&directory=out")
        .await;
    assert_eq!(r.status, 403);
    assert!(!s.root.outside("outdir/x").exists());

    let r = s
        .post_with("/", &[SAFE_AUTH], "item_type=file&name=keep.txt")
        .await;
    assert_eq!(
        (r.status, r.text().as_str()),
        (409, "Conflict: File already exists.")
    );
    assert_eq!(s.root.read("keep.txt"), "data");

    if !link(&s.root, "dang", &s.root.outside("new.txt")) {
        return;
    }
    let r = s
        .post_with("/", &[SAFE_AUTH], "item_type=file&name=dang")
        .await;
    assert_eq!(r.status, 409);
    assert!(!s.root.outside("new.txt").exists());

    let r = s
        .post_with("/", &[SAFE_AUTH], "item_type=file&name=fresh.txt")
        .await;
    assert_eq!(r.status, 302);
    assert!(s.root.exists("fresh.txt"));
}

#[tokio::test]
async fn plain_mode_overwrites_existing() {
    let Some(s) = site(false).await else { return };
    let r = s
        .post_with("/", &[SAFE_AUTH], "item_type=file&name=keep.txt")
        .await;
    assert_eq!(r.status, 302);
    assert_eq!(s.root.read("keep.txt"), "");
}
