#![cfg(windows)]
//! `--safe-mode` over HTTP with Windows path forms: backslash, drive, ADS stream, case.
mod support;

use support::{SAFE_AUTH, Site, safe_site};

const FILES: &[(&str, &str)] = &[("hello.txt", "hello"), ("sub/inner.txt", "inner")];

async fn site() -> Site {
    safe_site("admin", FILES).await
}

async fn status(site: &Site, target: &str) -> u16 {
    site.get_with(target, &[SAFE_AUTH]).await.status
}

#[tokio::test]
async fn escapes_and_foreign_forms_are_forbidden() {
    let site = site().await;
    for target in [
        "/..%5c..%5cWindows%5cwin.ini",
        "/sub%5c..%5c..%5cx",
        "/C:/Windows/win.ini",
        "/hello.txt:stream",
    ] {
        assert_eq!(status(&site, target).await, 403, "GET {target}");
    }
}

#[tokio::test]
async fn backslash_and_case_variants_are_served() {
    let site = site().await;
    for (target, body) in [
        ("/sub%5cinner.txt", "inner"),
        ("/HELLO.TXT", "hello"),
        ("/SUB/INNER.TXT", "inner"),
    ] {
        let r = site.get_with(target, &[SAFE_AUTH]).await;
        assert_eq!(r.status, 200, "GET {target}");
        assert_eq!(r.text(), body, "GET {target}");
    }
}

#[tokio::test]
async fn post_directory_escape_is_forbidden() {
    let site = site().await;
    for dir in ["..%5c..", "C:"] {
        let form = format!("item_type=file&directory={dir}&name=x");
        let r = site.post_with("/", &[SAFE_AUTH], &form).await;
        assert_eq!(r.status, 403, "directory={dir}");
        assert!(
            !site.root.exists("x"),
            "directory={dir}: file created in root"
        );
    }
}
