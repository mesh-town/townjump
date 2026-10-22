// Connection string scenarios through the public crate API: the server prints the URL, the client parses it and builds an identity.
// Values are from key-derivation.json and keyderivation-vectors.json (key `abc`, randomness `00 01 … 1f`).
use townjump_keys::{
    ClientIdentity, KeysError, ParsedUrl, ServerKeys, client_identity, parse_url, server_keys,
};
use townjump_testkit::fixed_random;

mod private_server {
    use super::*;

    #[test]
    fn url_carries_the_secret_and_the_private_mark() {
        let server = derive(Some("abc"), true);
        assert_eq!(server.url, "hs://s000abc");
        assert_eq!(parse_url(&server.url), parsed("abc", Some(true)));
    }

    #[test]
    fn client_reading_the_url_gets_the_server_pair() {
        let server = derive(Some("abc"), true);
        let (key_pair, seed_z32) = private(&read_url(&server.url, false));
        assert_eq!(key_pair, server.key_pair, "server recognises client by key");
        assert_eq!(seed_z32, server.server_key, "same seed z32 on both sides");
        assert_eq!(
            seed_z32,
            "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso"
        );
    }

    #[test]
    fn generated_key_url_reads_back_into_the_server_pair() {
        let server = derive(None, true);
        let key = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        assert_eq!(server.url, format!("hs://s000{key}"));
        let (key_pair, _) = private(&read_url(&server.url, false));
        assert_eq!(key_pair, server.key_pair);
    }
}

mod public_server {
    use super::*;

    const RANDOM_PUBLIC_Z32: &str = "yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";
    const ABC_PUBLIC_Z32: &str = "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y";

    #[test]
    fn url_has_no_private_mark_and_zeros_do_not_force_the_mode() {
        let server = derive(None, false);
        assert_eq!(server.url, format!("hs://0000{RANDOM_PUBLIC_Z32}"));
        // the mode is not derived from the URL, the caller's flag decides
        assert_eq!(parse_url(&server.url), parsed(RANDOM_PUBLIC_Z32, None));
    }

    #[test]
    fn client_reading_the_url_gets_the_server_public_key() {
        let server = derive(None, false);
        let got = public(&read_url(&server.url, false));
        assert_eq!(got, server.key_pair.public_key.to_vec());
        let want = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
        assert_eq!(hex::encode(got), want);
    }

    #[test]
    fn secret_of_a_public_server_with_key_is_not_in_the_url() {
        let server = derive(Some("abc"), false);
        assert_eq!(server.url, format!("hs://0000{ABC_PUBLIC_Z32}"));
        let got = public(&read_url(&server.url, false));
        let want = "0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa";
        assert_eq!(hex::encode(got), want);
    }

    #[test]
    fn same_url_with_the_private_flag_is_read_as_a_private_secret() {
        // the client `--public` flag enables private mode, while the URL carries no mark - the flag decides
        // (key-derivation.json, record 59: the same URL, `secure: true`)
        let server = derive(None, false);
        let (_, seed_z32) = private(&read_url(&server.url, true));
        assert_eq!(
            seed_z32,
            "eezo1444k38ng4o7rcb1q1qyapyjjg1c8ezi1wdgaznc57ey3zro"
        );
    }
}

/// How the facade reads a URL: the mode from `parse_url`, and if "not set" - from the flag (`data.secure ?? opts.secure`).
fn read_url(url: &str, flag: bool) -> Result<ClientIdentity, KeysError> {
    let data = parse_url(url);
    client_identity(&data.key, data.secure.unwrap_or(flag))
}

fn derive(key: Option<&str>, secure: bool) -> ServerKeys {
    server_keys(key, secure, &mut fixed_random())
}

fn parsed(key: &str, secure: Option<bool>) -> ParsedUrl {
    ParsedUrl {
        key: key.to_string(),
        secure,
    }
}

fn private(got: &Result<ClientIdentity, KeysError>) -> (townjump_keys::KeyPair, String) {
    match got {
        Ok(ClientIdentity::Private { key_pair, seed_z32 }) => (key_pair.clone(), seed_z32.clone()),
        other => panic!("expected Private, got {other:?}"),
    }
}

fn public(got: &Result<ClientIdentity, KeysError>) -> Vec<u8> {
    match got {
        Ok(ClientIdentity::Public { public_key }) => public_key.clone(),
        other => panic!("expected Public, got {other:?}"),
    }
}
