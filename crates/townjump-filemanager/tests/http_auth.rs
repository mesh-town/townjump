//! Whole server: authentication and the "decode before password" order
//! (authentication).
mod support;

use support::{ADMIN_AUTH, Setup, setup};

mod without_authorization_header {
    use super::*;

    // Authentication: without Authorization → 401 with a Basic realm="Filemanager" challenge.
    #[tokio::test]
    async fn answers_401_with_basic_challenge() {
        // given
        let site = setup(Setup::default()).await;
        // when
        let reply = site.get_with("/", &[]).await;
        // then
        assert_eq!(reply.status, 401);
        assert_eq!(
            reply.header("WWW-Authenticate"),
            Some(r#"Basic realm="Filemanager""#)
        );
        assert_eq!(reply.text(), "Authentication required.");
    }
}

mod basic_with_default_credentials {
    use super::*;

    // `Basic YWRtaW46YWRtaW4=` (admin:admin, livefiles defaults) → 200.
    #[tokio::test]
    async fn is_let_in() {
        let site = setup(Setup::default()).await;

        let reply = site.get_with("/", &[ADMIN_AUTH]).await;

        assert_eq!(reply.status, 200);
    }
}

mod wrong_password {
    use super::*;

    // Control for the 200 cases: the credentials themselves are compared (route/mod.rs `authenticate`).
    #[tokio::test]
    async fn is_denied_with_401() {
        let site = setup(Setup::default()).await;

        let reply = site
            .get_with("/", &[("Authorization", "Basic YWRtaW46d3Jvbmc=")])
            .await;

        assert_eq!(reply.status, 401, "admin:wrong must not pass");
    }
}

mod bearer_scheme_with_valid_credentials {
    use super::*;

    // The word before the space is not checked — `Bearer YWRtaW46YWRtaW4=` → 200.
    #[tokio::test]
    async fn is_let_in_because_scheme_word_is_ignored() {
        let site = setup(Setup::default()).await;

        let reply = site
            .get_with("/", &[("Authorization", "Bearer YWRtaW46YWRtaW4=")])
            .await;

        assert_eq!(reply.status, 200);
    }
}

mod scheme_word_without_credentials {
    use super::*;

    // `Basic` without a second word → TypeError ERR_INVALID_ARG_TYPE crash, not a byte of response.
    #[tokio::test]
    async fn crashes_with_invalid_arg_type_and_sends_no_bytes() {
        // given
        let site = setup(Setup::default()).await;
        // when
        let sent = site.raw_get("/", &[("Authorization", "Basic")]).await;
        let crashes = site.crashes().await;
        // then
        assert!(
            sent.is_empty(),
            "expected 0 bytes, got {:?}",
            String::from_utf8_lossy(&sent)
        );
        assert_eq!(crashes.len(), 1, "one crash: {crashes:?}");
        assert!(
            crashes[0].starts_with("TypeError [ERR_INVALID_ARG_TYPE]"),
            "crash text: {:?}",
            crashes[0]
        );
    }
}

mod credentials_with_extra_colon_part {
    use super::*;

    // Server a/b, credentials `a:b:c` (base64 `YTpiOmM=`) → 200: only [0] and [1] are compared.
    #[tokio::test]
    async fn are_let_in_because_only_first_two_parts_are_compared() {
        let site = setup(Setup {
            username: Some("a"),
            password: Some("b"),
            ..Default::default()
        })
        .await;

        let reply = site
            .get_with("/", &[("Authorization", "Basic YTpiOmM=")])
            .await;

        assert_eq!(reply.status, 200);
    }
}

mod malformed_percent_in_url {
    use super::*;

    // `GET /%` without a password → crash `URIError: URI malformed` (decode before auth, not 401).
    #[tokio::test]
    async fn crashes_with_uri_error_before_password_check() {
        // given
        let site = setup(Setup::default()).await;
        // when
        let sent = site.raw_get("/%", &[]).await;
        let crashes = site.crashes().await;
        // then
        assert!(
            sent.is_empty(),
            "expected 0 bytes (not 401), got {:?}",
            String::from_utf8_lossy(&sent)
        );
        assert_eq!(crashes, ["URIError: URI malformed"]);
    }
}
