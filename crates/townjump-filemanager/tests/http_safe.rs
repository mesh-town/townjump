//! Whole server under `--safe-mode`: startup, Host, Origin, methods, role, crashes, HTML, Location.
mod support;

use support::{Reply, SAFE_AUTH, Setup, TempRoot, raw, safe_site, setup};
use townjump_filemanager::{Filemanager, FilemanagerError, FilemanagerOpts};
use townjump_jsval::JsValue;

const AUTH_LINE: &str = "Authorization: Basic YWRtaW46czNjcmV0";

fn opts(root: &TempRoot, safe: bool, creds: Option<(&str, &str)>) -> FilemanagerOpts {
    let text = |s: &str| Some(JsValue::String(s.into()));
    FilemanagerOpts {
        path: text(&root.path()),
        username: creds.and_then(|c| text(c.0)),
        password: creds.and_then(|c| text(c.1)),
        safe_mode: safe,
        ..Default::default()
    }
}

mod start {
    use super::*;

    #[test]
    fn default_credentials_are_refused() {
        let root = TempRoot::new();
        let err = Filemanager::new(opts(&root, true, None)).err();
        assert_eq!(err, Some(FilemanagerError::DefaultCredentials));
        let err = Filemanager::new(opts(&root, true, Some(("admin", "admin")))).err();
        let err = err.expect("rejection");
        assert_eq!(
            err.to_string(),
            "Error: --safe-mode refuses the default password; set --password (and --username)"
        );
        let user_only = FilemanagerOpts {
            password: None,
            ..opts(&root, true, Some(("u", "x")))
        };
        assert_eq!(
            Filemanager::new(user_only).err(),
            Some(FilemanagerError::DefaultCredentials)
        );
        let err = Filemanager::new(opts(&root, true, Some(("u", "admin")))).err();
        assert_eq!(err, Some(FilemanagerError::DefaultCredentials));
    }

    #[test]
    fn explicit_credentials_and_plain_mode_start() {
        let root = TempRoot::new();
        assert!(Filemanager::new(opts(&root, true, Some(("admin", "s3cret")))).is_ok());
        assert!(Filemanager::new(opts(&root, false, None)).is_ok());
    }
}

mod host {
    use super::*;

    async fn get_with_host(site: &support::Site, host: Option<&str>, auth: bool) -> Reply {
        let mut req = match host {
            Some(h) => format!("GET / HTTP/1.1\r\nHost: {h}\r\n"),
            None => "GET / HTTP/1.0\r\n".to_owned(),
        };
        if auth {
            req.push_str(AUTH_LINE);
            req.push_str("\r\n");
        }
        req.push_str("Connection: close\r\n\r\n");
        Reply::from_bytes(&raw(site.addr, req.as_bytes()).await)
    }

    #[tokio::test]
    async fn foreign_host_is_forbidden() {
        let site = safe_site("admin", &[]).await;
        let r = get_with_host(&site, Some("evil.example"), true).await;
        assert_eq!(r.status, 403);
        assert_eq!(r.text(), "Forbidden: Host not allowed.");
    }

    #[tokio::test]
    async fn local_hosts_and_missing_host_pass() {
        let site = safe_site("admin", &[]).await;
        for h in [Some("127.0.0.1:5409"), Some("host.local:8989"), None] {
            assert_eq!(get_with_host(&site, h, true).await.status, 200, "{h:?}");
        }
    }

    #[tokio::test]
    async fn unauthorized_client_still_sees_401() {
        let site = safe_site("admin", &[]).await;
        assert_eq!(
            get_with_host(&site, Some("evil.example"), false)
                .await
                .status,
            401
        );
    }
}

mod origin {
    use super::*;

    #[tokio::test]
    async fn cross_origin_post_is_forbidden() {
        let site = safe_site("admin", &[]).await;
        let h = [SAFE_AUTH, ("Origin", "http://evil.example")];
        let r = site.post_with("/", &h, "item_type=file&name=x").await;
        assert_eq!(r.status, 403);
        assert_eq!(r.text(), "Forbidden: Cross-origin request.");
        assert!(!site.root.exists("x"));
    }

    #[tokio::test]
    async fn same_origin_and_no_origin_post_create() {
        let site = safe_site("admin", &[]).await;
        let h = [SAFE_AUTH, ("Origin", "http://localhost")];
        let r = site.post_with("/", &h, "item_type=file&name=x").await;
        assert_eq!(r.status, 302);
        assert!(site.root.exists("x"));
        let r = site
            .post_with("/", &[SAFE_AUTH], "item_type=file&name=y")
            .await;
        assert_eq!(r.status, 302);
        assert!(site.root.exists("y"));
    }
}

mod methods {
    use super::*;
    use support::request;

    #[tokio::test]
    async fn put_gets_405_with_allow() {
        let site = safe_site("admin", &[]).await;
        let r = Reply::from_bytes(&raw(site.addr, &request("PUT", "/", &[SAFE_AUTH], b"")).await);
        assert_eq!(r.status, 405);
        assert_eq!(r.header("Allow"), Some("GET, POST"));
        assert_eq!(r.text(), "Method Not Allowed");
    }

    #[tokio::test]
    async fn head_gets_405_without_body() {
        let site = safe_site("admin", &[]).await;
        let r = Reply::from_bytes(&raw(site.addr, &request("HEAD", "/", &[SAFE_AUTH], b"")).await);
        assert_eq!(r.status, 405);
        assert!(r.body.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_delete_is_401() {
        let site = safe_site("admin", &[]).await;
        let r = Reply::from_bytes(&raw(site.addr, &request("DELETE", "/", &[], b"")).await);
        assert_eq!(r.status, 401);
    }
}

mod role {
    use super::*;

    #[tokio::test]
    async fn user_cannot_post_files() {
        let site = safe_site("user", &[]).await;
        let r = site
            .post_with("/", &[SAFE_AUTH], "item_type=file&name=x")
            .await;
        assert_eq!(r.status, 403);
        assert_eq!(r.text(), "Forbidden: Only admin users can create files.");
        assert!(!site.root.exists("x"));
        let g = site.get_with("/", &[SAFE_AUTH]).await;
        assert_eq!(g.status, 200);
        assert!(!g.text().contains("item_type"));
    }
}

mod crashes {
    use super::*;

    #[tokio::test]
    async fn bad_percent_url_is_400_and_server_lives() {
        let site = safe_site("admin", &[]).await;
        let r = site.get_with("/%E0%A4%A", &[SAFE_AUTH]).await;
        assert_eq!((r.status, r.text().as_str()), (400, "Bad Request"));
        assert!(site.crashes_now().is_empty());
        assert_eq!(site.get_with("/", &[SAFE_AUTH]).await.status, 200);
    }

    #[tokio::test]
    async fn one_word_authorization_is_400() {
        let site = safe_site("admin", &[]).await;
        let r = site.get_with("/", &[("Authorization", "Basic")]).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn bad_range_is_400() {
        let site = safe_site("admin", &[("t.txt", "0123456789")]).await;
        let r = site
            .get_with("/t.txt", &[SAFE_AUTH, ("Range", "bytes=5-2")])
            .await;
        assert_eq!(r.status, 400);
        assert!(site.crashes_now().is_empty());
    }

    #[tokio::test]
    async fn empty_cyrillic_file_is_served() {
        let site = safe_site("admin", &[("πυστο.txt", "")]).await;
        let r = site
            .get_with("/%CF%80%CF%85%CF%83%CF%84%CE%BF.txt", &[SAFE_AUTH])
            .await;
        assert_eq!(r.status, 200);
        let d = r.header("Content-Disposition").expect("header");
        assert!(d.contains("filename*=UTF-8''%CF%80"), "{d}");
    }
}

mod html {
    use super::*;

    /// Names are shown escaped: every `raw` text is absent and every `escaped` text is present.
    async fn check_escaped(
        files: &'static [(&'static str, &'static str)],
        raw: &[&str],
        escaped: &[&str],
    ) {
        let site = safe_site("admin", files).await;
        let body = site.get_with("/", &[SAFE_AUTH]).await.text();
        for r in raw {
            assert!(!body.contains(r), "raw {r:?} in {body}");
        }
        for e in escaped {
            assert!(body.contains(e), "missing {e:?} in {body}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn names_are_escaped() {
        check_escaped(
            &[("\"><img src=x onerror=1>", "a"), ("<b>d/f", "b")],
            &["\"><img", "<b>d"],
            &["&quot;&gt;&lt;img", "&lt;b&gt;d</option>"],
        )
        .await;
    }

    /// Windows forbids `<>"` in names; `&` and `'` are legal there and are escaped as well.
    #[tokio::test]
    async fn names_with_amp_and_quote_are_escaped() {
        check_escaped(
            &[("a&b'c.txt", "a"), ("d&e'f/g", "b")],
            &["a&b'c", "d&e'f"],
            &["a&amp;b&#039;c.txt", "d&amp;e&#039;f</option>"],
        )
        .await;
    }

    const XSS: &str = "/x%22%3E%3Csvg%20onload%3Dalert(1)%3E%2F..";

    #[tokio::test]
    async fn url_in_form_is_escaped() {
        let site = safe_site("admin", &[]).await;
        let body = site.get_with(XSS, &[SAFE_AUTH]).await.text();
        assert!(!body.contains("<svg onload"), "{body}");
        assert!(body.contains("&lt;svg"));
    }

    #[tokio::test]
    async fn plain_mode_keeps_js_parity() {
        let site = setup(Setup {
            role: Some("admin"),
            ..Default::default()
        })
        .await;
        let body = site.get(XSS).await.text();
        assert!(body.contains("\"><svg onload=alert(1)>"), "{body}");
    }
}

mod location {
    use super::*;

    const DIR: &str = "/%CF%80%CE%B1%CF%80%CE%BA%CE%B1/";

    #[tokio::test]
    async fn redirect_is_percent_encoded() {
        let site = safe_site("admin", &[("παπκα/f.txt", "x")]).await;
        let r = site
            .post_with(DIR, &[SAFE_AUTH], "item_type=file&name=n")
            .await;
        assert_eq!(r.status, 302);
        assert_eq!(r.header("Location"), Some(DIR));
    }

    #[tokio::test]
    async fn plain_mode_is_500_found() {
        let site = setup(Setup {
            role: Some("admin"),
            files: &[("παπκα/f.txt", "x")],
            ..Default::default()
        })
        .await;
        let r = site.post(DIR, "item_type=file&name=n").await;
        assert_eq!(r.status, 500);
    }
}
