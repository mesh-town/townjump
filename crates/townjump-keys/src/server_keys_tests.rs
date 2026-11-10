use super::*;

fn rnd(buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = i as u8;
    }
}

const RND_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

#[test]
fn key_given_private_does_not_call_random() {
    let mut calls = 0;
    let k = server_keys(Some("abc"), true, &mut |_| calls += 1);
    assert_eq!(calls, 0);
    assert_eq!(k.key, "abc");
    assert_eq!(k.info_key, "abc");
    assert_eq!(k.url, "hs://s000abc");
    assert_eq!(
        k.server_key,
        "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso"
    );
    assert_eq!(k.seed_hex.as_deref(), Some(k.generated_seed_hex.as_str()));
}

#[test]
fn key_given_public_hides_key_in_url() {
    let k = server_keys(Some("abc"), false, &mut |_| panic!("no random"));
    assert_eq!(
        k.server_key,
        "bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y"
    );
    assert_eq!(k.info_key, k.server_key);
    assert_eq!(k.url, format!("hs://0000{}", k.server_key));
    assert_eq!(k.key, "abc");
}

#[test]
fn no_key_private_generates_hex_key_once() {
    let mut calls = 0;
    let k = server_keys(None, true, &mut |b| {
        calls += 1;
        rnd(b);
    });
    assert_eq!(calls, 1);
    assert_eq!(k.key, RND_HEX);
    assert_eq!(k.url, format!("hs://s000{RND_HEX}"));
    assert_eq!(
        k.server_key,
        "p1dcpksf9c1m37q31qqmxi6ictqq8fya6tr6yq3gfzkxwffwzrio"
    );
    assert_eq!(
        k.seed_hex.as_deref(),
        Some("6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b")
    );
}

#[test]
fn no_key_public_uses_random_bytes_unhashed() {
    let mut calls = 0;
    let k = server_keys(None, false, &mut |b| {
        calls += 1;
        rnd(b);
    });
    assert_eq!(calls, 1);
    assert_eq!(k.key, "");
    assert_eq!(k.seed_hex, None);
    assert_eq!(k.generated_seed_hex, RND_HEX);
    assert_eq!(
        k.server_key,
        "yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy"
    );
    assert_eq!(
        &k.key_pair.secret_key[..32],
        &hex::decode(RND_HEX).unwrap()[..]
    );
}

#[test]
fn empty_key_is_no_key() {
    let a = server_keys(Some(""), true, &mut rnd);
    let b = server_keys(None, true, &mut rnd);
    assert_eq!(a, b);
    assert_eq!(
        server_keys(Some(""), false, &mut rnd),
        server_keys(None, false, &mut rnd)
    );
}
