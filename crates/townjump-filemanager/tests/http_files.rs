//! Whole server: directory listing and file serving
//! (listing, files).
mod support;

use support::{ADMIN_AUTH, Setup, assert_in_order, setup};

const LISTING_FILES: &[(&str, &str)] = &[("b.txt", "abc"), ("a/x.bin", "0123456789")];

mod root_listing {
    use super::*;

    // Listing: directory `a/` (x.bin, 10 bytes) comes before file `b.txt` (3 bytes), sizes in the rows.
    #[tokio::test]
    async fn lists_directory_before_file_each_with_size() {
        // given
        let site = setup(Setup {
            files: LISTING_FILES,
            ..Default::default()
        })
        .await;
        // when
        let reply = site.get("/").await;
        // then
        assert_eq!(reply.status, 200);
        assert_in_order(
            &reply.text(),
            &[
                r#"<a href="/a">a</a>"#,
                "10 Bytes",
                r#"<a href="/b.txt">b.txt</a>"#,
                "3 Bytes",
            ],
        );
    }
}

mod user_role {
    use super::*;

    // Role user — no create form in the body (`item_type`).
    #[tokio::test]
    async fn listing_has_no_create_form() {
        let site = setup(Setup {
            files: LISTING_FILES,
            ..Default::default()
        })
        .await;

        let reply = site.get("/").await;

        assert!(
            !reply.text().contains("item_type"),
            "user must not have a create form"
        );
    }
}

mod admin_role {
    use super::*;

    // Role admin — the body has the create form (`item_type`).
    #[tokio::test]
    async fn listing_has_create_form() {
        let site = setup(Setup {
            role: Some("admin"),
            files: LISTING_FILES,
            ..Default::default()
        })
        .await;

        let reply = site.get("/").await;

        assert!(
            reply.text().contains("item_type"),
            "admin must have a create form"
        );
    }
}

mod existing_file {
    use super::*;

    // Files: `GET /b.txt` → 200, Content-Length 3, Content-Disposition with the file name.
    #[tokio::test]
    async fn is_sent_whole_with_length_and_attachment_name() {
        // given
        let site = setup(Setup {
            files: &[("b.txt", "abc")],
            ..Default::default()
        })
        .await;
        // when
        let reply = site.get("/b.txt").await;
        // then
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("Content-Length"), Some("3"));
        assert_eq!(
            reply.header("Content-Disposition"),
            Some(r#"attachment; filename="b.txt""#)
        );
        assert_eq!(reply.text(), "abc");
    }
}

mod range_from_offset_inside_file {
    use super::*;

    // Range `bytes=1-` on a 3-byte file → 206, `Content-Range: bytes 1-2/3`.
    #[tokio::test]
    async fn gets_206_with_content_range_and_tail() {
        let site = setup(Setup {
            files: &[("b.txt", "abc")],
            ..Default::default()
        })
        .await;

        let reply = site
            .get_with("/b.txt", &[ADMIN_AUTH, ("Range", "bytes=1-")])
            .await;

        assert_eq!(reply.status, 206);
        assert_eq!(reply.header("Content-Range"), Some("bytes 1-2/3"));
        assert_eq!(reply.text(), "bc");
    }
}

mod range_starting_at_file_size {
    use super::*;

    // Range `bytes=3-` on a 3-byte file → 416, `Content-Range: bytes */3`, empty body.
    #[tokio::test]
    async fn gets_416_with_size_in_content_range() {
        let site = setup(Setup {
            files: &[("b.txt", "abc")],
            ..Default::default()
        })
        .await;

        let reply = site
            .get_with("/b.txt", &[ADMIN_AUTH, ("Range", "bytes=3-")])
            .await;

        assert_eq!(reply.status, 416);
        assert_eq!(reply.header("Content-Range"), Some("bytes */3"));
        assert!(reply.body.is_empty(), "the 416 body must be empty");
    }
}

mod file_name_outside_latin1 {
    use super::*;

    // With a nonzero Content-Length
    // Node writes Content-Disposition via Buffer.from(v, 'latin1') — char -> low byte.
    // U+0444 -> `D`.
    #[tokio::test]
    async fn truncates_name_to_low_bytes_when_body_is_not_empty() {
        // given
        let site = setup(Setup {
            files: &[("ń.txt", "x")],
            ..Default::default()
        })
        .await;
        // when
        let reply = site.get("/%C5%84.txt").await;
        // then
        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.header("Content-Disposition"),
            Some(r#"attachment; filename="D.txt""#)
        );
        assert!(site.crashes().await.is_empty());
    }

    // Empty file: `_contentLength` = 0 is falsy, no truncation, the regexp sees U+0444 -> crash, no response bytes.
    #[tokio::test]
    async fn crashes_for_empty_file() {
        // given
        let site = setup(Setup {
            files: &[("φ.txt", "")],
            ..Default::default()
        })
        .await;
        // when
        let sent = site.raw_get("/%CF%86.txt", &[ADMIN_AUTH]).await;
        let crashes = site.crashes().await;
        // then
        assert!(
            sent.is_empty(),
            "expected 0 bytes, got {:?}",
            String::from_utf8_lossy(&sent)
        );
        assert_eq!(
            crashes,
            [
                r#"TypeError [ERR_INVALID_CHAR]: Invalid character in header content ["Content-Disposition"]"#
            ]
        );
    }
}

mod missing_path {
    use super::*;

    // `GET /nope` → 404 `File Not Found`.
    #[tokio::test]
    async fn gets_404_file_not_found() {
        let site = setup(Setup::default()).await;

        let reply = site.get("/nope").await;

        assert_eq!(reply.status, 404);
        assert_eq!(reply.text(), "File Not Found");
    }
}
