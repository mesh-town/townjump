// Tests for `fingerprint`: the decision on secrets in the log - the first min(8, n) characters
// by `char`, then one `*` for each following character. JS does not do this (it prints the secret in full), no vectors.
use super::*;

mod short_secret {
    use super::*;

    #[test]
    fn empty_stays_empty() {
        assert_eq!(fingerprint(""), "");
    }

    #[test]
    fn up_to_eight_chars_is_shown_whole() {
        assert_eq!(fingerprint("abc"), "abc");
        assert_eq!(fingerprint("abcde"), "abcde");
        assert_eq!(fingerprint("abcdefgh"), "abcdefgh");
    }
}

mod long_secret {
    use super::*;

    #[test]
    fn ninth_char_becomes_one_star() {
        assert_eq!(fingerprint("abcdefghi"), "abcdefgh*");
    }

    #[test]
    fn each_further_char_adds_one_star() {
        for len in [52usize, 64] {
            let secret = "k".repeat(len);
            let want = format!("kkkkkkkk{}", "*".repeat(len - 8));
            assert_eq!(fingerprint(&secret), want, "length {len}");
        }
    }

    #[test]
    fn key_from_the_vector_shows_only_its_first_eight_chars() {
        // K from keyderivation-vectors.json: 64 hex characters
        let secret = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let want = format!("00010203{}", "*".repeat(56));
        assert_eq!(fingerprint(secret), want);
    }

    #[test]
    fn z32_seed_from_the_vector_shows_only_its_first_eight_chars() {
        // `z32(sha256(K))` is what the client would print in `Connecting to key`; 52 characters
        let secret = "zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso";
        let want = format!("zjhbpxhx{}", "*".repeat(44));
        assert_eq!(fingerprint(secret), want);
    }

    #[test]
    fn full_secret_is_never_in_the_result() {
        for len in 9..=70usize {
            let secret: String = ('a'..='z').cycle().take(len).collect();
            let got = fingerprint(&secret);
            assert_ne!(got, secret, "length {len}: the secret is fully visible");
            assert!(got.ends_with('*'), "length {len}: {got}");
            assert!(
                !got.contains(&secret[8..]),
                "length {len}: the secret tail is visible"
            );
        }
    }
}

mod non_ascii_secret {
    use super::*;

    #[test]
    fn counts_chars_not_bytes() {
        assert_eq!(fingerprint("αβωγδεζθι"), "αβωγδεζθ*");
        assert_eq!(fingerprint("αβωγδεζθ"), "αβωγδεζθ");
    }

    #[test]
    fn emoji_is_one_char() {
        assert_eq!(fingerprint("😀😀😀😀😀😀😀😀😀😀"), "😀😀😀😀😀😀😀😀**");
    }

    #[test]
    fn combining_mark_is_a_char_of_its_own() {
        // `char`, not a grapheme: `e` + U+0301 take two of the eight places
        let secret = "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}";
        assert_eq!(fingerprint(secret), "e\u{301}e\u{301}e\u{301}e\u{301}**");
    }
}
