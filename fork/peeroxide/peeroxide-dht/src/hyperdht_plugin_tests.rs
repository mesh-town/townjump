//! Vectors: testvectors/deltas/codec.json `d1.pluginRequest` (JS hyperdht 6.34.0).
use super::*;

fn req(value: Option<Vec<u8>>) -> PluginRequest {
    PluginRequest {
        plugin: "x".into(),
        version: 1,
        command: 2,
        value,
    }
}

#[test]
fn encodes_three_vectors() {
    assert_eq!(hex::encode(encode_plugin_request(&req(None))), "0178010200");
    assert_eq!(
        hex::encode(encode_plugin_request(&req(Some(vec![])))),
        "017801020100"
    );
    assert_eq!(
        hex::encode(encode_plugin_request(&req(Some(vec![0xaa, 0xbb])))),
        "017801020102aabb"
    );
}

#[test]
fn decodes_three_vectors() {
    let d = |h: &str| decode_plugin_request(&hex::decode(h).unwrap()).unwrap();
    assert_eq!(d("0178010200"), req(None));
    assert_eq!(d("017801020100"), req(Some(vec![])));
    assert_eq!(d("017801020102aabb"), req(Some(vec![0xaa, 0xbb])));
}

#[test]
fn truncated_value_is_error() {
    assert!(decode_plugin_request(&hex::decode("017801020103aa").unwrap()).is_err());
}
