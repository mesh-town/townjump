// Tests of `cli::qr`: `qr_small` byte for byte as qrcode-terminal 0.12.0 (`small: true`, level L) — the QR from the CLI matrix banners
// and three references captured by running JS (`node -e`): small version 1, a short connection string, UTF-8 in the string.
use super::*;
use townjump_testkit::vectors::load_value;

mod server_banner_urls_match_js {
    use super::*;

    #[test]
    fn private_url_of_a_random_seed() {
        let url = "hs://s000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        assert_eq!(qr_small(url), banner_qr("live-private"));
    }

    #[test]
    fn public_url_from_the_z32_key() {
        let url = "hs://0000yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy";
        assert_eq!(qr_small(url), banner_qr("live-public"));
    }

    #[test]
    fn short_custom_key_gives_a_smaller_code() {
        let url = "hs://s000abcdefghijklmnopqrstuvwxyz0123456789ABCD";
        assert_eq!(qr_small(url), banner_qr("live-key40"));
    }

    #[test]
    fn hex_custom_key() {
        let url = "hs://s000a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
        assert_eq!(qr_small(url), banner_qr("live-key-hex-b"));
    }
}

mod small_literals_from_js {
    use super::*;

    #[test]
    fn one_letter_gives_the_version_1_code_of_12_rows() {
        let rows = [
            "▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄",
            "█ ▄▄▄▄▄ █▀▀ ▄ █ ▄▄▄▄▄ █",
            "█ █   █ █▀▀▄▄ █ █   █ █",
            "█ █▄▄▄█ █▀▄█▀▄█ █▄▄▄█ █",
            "█▄▄▄▄▄▄▄█▄▀▄█▄█▄▄▄▄▄▄▄█",
            "█   ▄ █▄▄▄█▀ ▀ ▀▄█ █▄██",
            "███▄ ██▄ █▀▀▀ ▀ ▄█▀█▀██",
            "█▄▄▄▄█▄▄▄  ▄█ ██ ██ █▀█",
            "█ ▄▄▄▄▄ █▄  ▄█▄█▀ ▄  ██",
            "█ █   █ █ ▄▀ ▀ ▀█▄ ▄███",
            "█ █▄▄▄█ █ ▀▄▀ ▀ ▄█▀████",
            "█▄▄▄▄▄▄▄█▄█▄█▄██▄██▄▄██",
        ];
        assert_eq!(qr_small("a"), console_log(&rows));
    }

    #[test]
    fn short_connection_string_gives_the_version_1_code() {
        let rows = [
            "▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄",
            "█ ▄▄▄▄▄ █▀▄█▀▄█ ▄▄▄▄▄ █",
            "█ █   █ █▀▄█▀██ █   █ █",
            "█ █▄▄▄█ █▀▀ ▄ █ █▄▄▄█ █",
            "█▄▄▄▄▄▄▄█▄█ ▀ █▄▄▄▄▄▄▄█",
            "█  ▄  █▄ ▄ ▀█ ▄█▄▀▄▀ ▀█",
            "█▄▄▀▄██▄  ▀▀▄███ █▄▀▄██",
            "██▄▄▄▄▄▄▄ ▀█▄▀▄▄▄▀▄██ █",
            "█ ▄▄▄▄▄ █▄▀█▀▄▀█ ██▀▄██",
            "█ █   █ █ ▀▀█▄█▄▄▀▀█▄▄█",
            "█ █▄▄▄█ █ ▀▀▄▄ █▄▀▄▀███",
            "█▄▄▄▄▄▄▄█▄███▄█▄█▄██▄██",
        ];
        assert_eq!(qr_small("hs://0000abc"), console_log(&rows));
    }

    #[test]
    fn non_ascii_url_is_encoded_as_utf8_bytes() {
        let rows = [
            "▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄",
            "█ ▄▄▄▄▄ ██▄ ▀▄▄█▄▄█ ▄▄▄▄▄ █",
            "█ █   █ █▄  ▄▀▄▄▄▀█ █   █ █",
            "█ █▄▄▄█ ██▀▄▄ ▄█▄ █ █▄▄▄█ █",
            "█▄▄▄▄▄▄▄█ ▀ ▀▄█ ▀▄█▄▄▄▄▄▄▄█",
            "█   ▀ ▄▄ ▄█   ██  ▄ ▀▀▀ █▀█",
            "█  █▀ ▀▄ █▄█▄██ ▄█▀▀▀▄▀██ █",
            "█▄▄▀██ ▄█▄▀█▄█▄ ▄██▀ ██▀█ █",
            "█▄▀▄  █▄ ▄▄▄▄▄▄█▄▄█▀▀▄ █ ▄█",
            "█▄█▄█▄█▄▄▀▄█▄█▄ ▄ ▄▄▄ █▀▀▀█",
            "█ ▄▄▄▄▄ █  ▄▄ ▄▄  █▄█ ▄█▄▄█",
            "█ █   █ █▄ ▄▄█▄   ▄ ▄  █▄██",
            "█ █▄▄▄█ █ █  █▄ ▄▀▀ ▀▄ █▀▄█",
            "█▄▄▄▄▄▄▄█▄█▄▄▄████▄▄█▄▄█▄▄█",
        ];
        assert_eq!(
            qr_small("hs://s000κλωξκλωξκλωξκλωξκλωξ"),
            console_log(&rows)
        );
    }
}

mod output_shape {
    use super::*;

    #[test]
    fn only_half_blocks_spaces_and_newlines_and_a_blank_line_at_the_end() {
        let out =
            qr_small("hs://s000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        assert!(
            out.chars().all(|c| "▀▄█ \n".contains(c)),
            "extra character in the QR"
        );
        assert!(
            out.ends_with("\n\n"),
            "console.log adds an empty line after the QR"
        );
        assert!(!out.ends_with("\n\n\n"), "exactly one empty line");
    }

    #[test]
    fn calls_do_not_share_state() {
        let first = qr_small("hs://0000abc");
        let _other = qr_small("hs://s000κλωξκλωξκλωξκλωξκλωξ");
        assert_eq!(
            qr_small("hs://0000abc"),
            first,
            "a repeated call gives the same"
        );
    }
}

/// What `console.log(qr)` prints when `qr` is lines each with `\n`: an empty line after.
fn console_log(rows: &[&str]) -> String {
    format!("{}\n\n", rows.join("\n"))
}

/// The tail of the server banner stdout after the bottom frame border is exactly the QR.
fn banner_qr(id: &str) -> String {
    let matrix = load_value("holesail-cli/cli-matrix.json");
    let case = matrix
        .as_array()
        .and_then(|all| all.iter().find(|c| c["id"] == id));
    let stdout = case.unwrap_or_else(|| panic!("no case {id} in the matrix"))["stdout"]
        .as_str()
        .expect("stdout");
    let end = stdout.rfind("┘\n").expect("bottom frame border") + "┘\n".len();
    stdout[end..].to_owned()
}
