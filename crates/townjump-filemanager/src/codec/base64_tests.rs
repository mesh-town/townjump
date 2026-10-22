use super::node_base64;

#[test]
fn decodes_like_node() {
    assert_eq!(node_base64("YWRtaW46YWRtaW4="), b"admin:admin");
    assert_eq!(node_base64("!!!!"), b"");
    assert_eq!(node_base64("YW Rt"), b"adm");
    assert_eq!(node_base64("YQ"), b"a");
    assert_eq!(node_base64("YQ=x"), b"a");
    assert_eq!(node_base64("Y"), b"");
    assert_eq!(node_base64("-_8"), [251, 255]);
    assert_eq!(node_base64("YWRt\naW4"), b"admin");
}
