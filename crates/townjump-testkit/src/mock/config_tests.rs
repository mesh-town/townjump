use super::*;

#[test]
fn default_is_strict() {
    let config = MockConfig::default();
    assert!(config.record.is_none());
    assert!(!config.fake_conn);
    assert!(!config.lenient_connect);
    assert!(config.canned(&[1; 32]).is_none());
}

#[test]
fn from_env_star_record() {
    let config =
        MockConfig::from_env_pairs([("MOCK_RECORD_FOR", "*"), ("MOCK_RECORD", "{\"a\":1}")]);
    assert!(config.lenient_connect);
    assert_eq!(config.record, Some((RecordFor::Any, b"{\"a\":1}".to_vec())));
    assert_eq!(config.canned(&[5; 32]), Some(&b"{\"a\":1}"[..]));
}

#[test]
fn from_env_key_record() {
    let key = [0xab; 32];
    let hex_key = hex::encode(key);
    let config =
        MockConfig::from_env_pairs([("MOCK_RECORD_FOR", hex_key.as_str()), ("MOCK_RECORD", "v")]);
    assert_eq!(config.record, Some((RecordFor::Key(key), b"v".to_vec())));
    assert!(config.canned(&key).is_some());
    assert!(config.canned(&[0xcd; 32]).is_none());
}

#[test]
fn from_env_rejects_bad_key_and_lone_vars() {
    let upper = "AB".repeat(32);
    let short = "ab".repeat(31) + "a";
    let cases: [Vec<(&str, &str)>; 5] = [
        vec![("MOCK_RECORD_FOR", upper.as_str()), ("MOCK_RECORD", "v")],
        vec![("MOCK_RECORD_FOR", short.as_str()), ("MOCK_RECORD", "v")],
        vec![("MOCK_RECORD", "v")],
        vec![("MOCK_RECORD_FOR", "*")],
        vec![("MOCK_RECORD_FOR", "*"), ("MOCK_RECORD", "")],
    ];
    for vars in cases {
        assert!(
            MockConfig::from_env_pairs(vars.clone()).record.is_none(),
            "{vars:?}"
        );
    }
}

#[test]
fn from_env_fake_conn_and_unrelated() {
    assert!(MockConfig::from_env_pairs([("MOCK_FAKE_CONN", "1")]).fake_conn);
    assert!(!MockConfig::from_env_pairs([("MOCK_FAKE_CONN", "")]).fake_conn);
    let unrelated = [("MOCK_TRACE", "/tmp/t"), ("MOCK_RANDOM_HEX", "00")];
    let config = MockConfig::from_env_pairs(unrelated);
    assert!(!config.fake_conn && config.record.is_none() && config.lenient_connect);
}
