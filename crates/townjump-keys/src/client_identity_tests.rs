use super::*;

#[test]
fn private_empty_key_is_hashed() {
    match client_identity("", true).unwrap() {
        ClientIdentity::Private { key_pair, seed_z32 } => {
            assert_eq!(
                seed_z32,
                "hqaceowa9oqbjgz56urj1573rou4hoxrc1p3gufr1scts6n1zbko"
            );
            assert_eq!(
                hex::encode(key_pair.public_key),
                "be907b4bac84fee5ce8811db2defc9bf0b2a2a2bbc3d54d8a2257ecd70441962"
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn public_decodes_without_length_check() {
    assert_eq!(
        client_identity("abc", false).unwrap(),
        ClientIdentity::Public {
            public_key: vec![0xc0, 0x58]
        }
    );
    assert_eq!(
        client_identity("", false).unwrap(),
        ClientIdentity::Public { public_key: vec![] }
    );
}

#[test]
fn public_errors_carry_char_and_position() {
    let e = client_identity("abcdefghijklmnopqrstuvwxyz012345", false).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Invalid character in base32 input: \"l\" at position 11"
    );
    let e = client_identity("123456", false).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Invalid character in base32 input: \"2\" at position 1"
    );
}

#[test]
fn private_never_fails_on_bad_alphabet() {
    assert!(client_identity("κλωξ-κλωξ", true).is_ok());
    assert!(client_identity(&"0".repeat(64), true).is_ok());
    assert!(client_identity(&"0".repeat(64), false).is_err());
}
