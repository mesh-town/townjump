//! Whole server: POST — creating folders and files
//! (POST).
mod support;

use support::{Setup, setup};

/// POST body limit (`conn.rs`): 10 MiB.
const BODY_LIMIT: usize = 10 * 1024 * 1024;

mod user_role_creates_folder {
    use super::*;

    // user + folder → 403 `Forbidden: Only admin users can create folders.`, no folder.
    #[tokio::test]
    async fn is_forbidden_with_admin_only_text() {
        // given
        let site = setup(Setup::default()).await;
        // when
        let reply = site.post("/", "item_type=folder&name=n").await;
        // then
        assert_eq!(reply.status, 403);
        assert_eq!(
            reply.text(),
            "Forbidden: Only admin users can create folders."
        );
        assert!(!site.root.exists("n"), "the folder must not be created");
    }
}

mod admin_role_creates_folder {
    use super::*;

    // admin + folder `n` → 302 `Location: /`, directory `n` created.
    #[tokio::test]
    async fn redirects_back_and_creates_the_directory() {
        let site = admin_site(&[]).await;

        let reply = site.post("/", "item_type=folder&name=n").await;

        assert_eq!(reply.status, 302);
        assert_eq!(reply.header("Location"), Some("/"));
        assert!(site.root.is_dir("n"), "directory n must be created");
    }
}

mod file_over_existing_file {
    use super::*;

    // file over a file with content → 302, the file is truncated.
    #[tokio::test]
    async fn redirects_and_truncates_the_file() {
        let site = admin_site(&[("f.txt", "content")]).await;

        let reply = site.post("/", "item_type=file&name=f.txt").await;

        assert_eq!(reply.status, 302);
        assert_eq!(site.root.read("f.txt"), "", "the file must become empty");
    }
}

mod form_without_name {
    use super::*;

    // without `name` → 400 (`Bad Request: Missing or invalid form data.`).
    #[tokio::test]
    async fn gets_400_bad_request() {
        let site = admin_site(&[]).await;

        let reply = site.post("/", "item_type=file").await;

        assert_eq!(reply.status, 400);
        assert_eq!(reply.text(), "Bad Request: Missing or invalid form data.");
    }
}

mod two_directory_fields {
    use super::*;

    // `directory=a&directory=b` → 500 `Internal Server Error`.
    #[tokio::test]
    async fn gets_500_internal_server_error() {
        let site = admin_site(&[]).await;

        let reply = site
            .post("/", "item_type=file&name=x&directory=a&directory=b")
            .await;

        assert_eq!(reply.status, 500);
        assert_eq!(reply.text(), "Internal Server Error");
    }
}

mod body_over_the_limit {
    use super::*;

    // body of 10 MiB + 1 → 413 or a drop (both acceptable; an RST mid-response gives a truncated 413 — also a drop); handle_post does not fire.
    #[tokio::test]
    async fn gets_413_or_connection_drop_and_creates_nothing() {
        // given
        let site = admin_site(&[]).await;
        let body = form_of_len("item_type=file&name=x&pad=", BODY_LIMIT + 1);
        // when
        let sent = site.raw_post("/", &body).await;
        // then
        let status = support::Reply::parse(&sent, true).map(|(reply, _)| reply.status);
        assert!(
            sent.is_empty() || status == Some(413) || sent.starts_with(b"HTTP/1.1 413"),
            "expected 413 or a drop, got {:?}",
            String::from_utf8_lossy(&sent[..sent.len().min(200)])
        );
        assert!(!site.root.exists("x"), "the file must not be created");
    }
}

mod escape_checked_on_final_path {
    use super::*;

    // Result `root/x` inside the root (as in JS): 302 and the file is created; root escapes — 403.
    #[tokio::test]
    async fn inner_dotdot_is_allowed_and_real_escapes_are_forbidden() {
        let site = admin_site(&[("a/keep", "")]).await;

        let reply = site.post("/", "item_type=file&directory=a&name=../x").await;
        assert_eq!(reply.status, 302);
        assert!(site.root.exists("x"), "file root/x must be created");

        let reply = site
            .post("/", "item_type=file&directory=../x&name=..")
            .await;
        assert_eq!(reply.status, 403);
        let reply = site.post("/", "item_type=file&name=../../x").await;
        assert_eq!(reply.status, 403);
    }
}

async fn admin_site(files: &'static [(&'static str, &'static str)]) -> support::Site {
    setup(Setup {
        role: Some("admin"),
        files,
        ..Default::default()
    })
    .await
}

/// Form body of exactly `len` bytes: `prefix` padded with `a` characters.
fn form_of_len(prefix: &str, len: usize) -> Vec<u8> {
    let mut body = prefix.as_bytes().to_vec();
    body.resize(len, b'a');
    body
}
