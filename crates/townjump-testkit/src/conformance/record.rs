// JS: hyperdht/lib/hyperdht.js `mutablePut`/`mutableGet` (seq and value) between different nodes.
// Related: setup.rs (`node`). Keys: 0x51 record, 0x52 record without seq, 0x53 reader and a key with no record.
use super::keys::key_pair;
use super::setup::node;
use super::wait::{ACCEPT_WAIT, within};
use townjump_dht::{Dht, DhtFactory, MutableRecord};

fn record(seq: u64, value: &[u8]) -> Option<MutableRecord> {
    Some(MutableRecord {
        seq,
        value: value.to_vec(),
    })
}

async fn put<D: Dht>(dht: &D, tag: u8, value: &[u8], seq: Option<u64>) -> u64 {
    let kp = key_pair(tag);
    let sent = dht.mutable_put(&kp, value, seq);
    let done = within(ACCEPT_WAIT, "record: put", sent).await;
    done.unwrap_or_else(|e| panic!("record: put ended with an error: {e}"))
}

async fn get<D: Dht>(dht: &D, pk: [u8; 32]) -> Option<MutableRecord> {
    let found = within(ACCEPT_WAIT, "record: get", dht.mutable_get(pk, Some(true))).await;
    found.unwrap_or_else(|e| panic!("record: get ended with an error: {e}"))
}

/// `put` returns the written `seq` (0 without `seq`); another node reads the record with the highest `seq`.
pub async fn mutable_record_seq<F: DhtFactory>(factory: &F) {
    let writer = node(factory, 0x51).await;
    let reader = node(factory, 0x53).await;
    assert_eq!(put(&writer, 0x51, b"v1", Some(1)).await, 1, "record: seq 1");
    assert_eq!(put(&writer, 0x51, b"v2", Some(2)).await, 2, "record: seq 2");
    assert_eq!(
        get(&reader, key_pair(0x51).public_key).await,
        record(2, b"v2"),
        "record: read v2"
    );
    assert_eq!(put(&writer, 0x52, b"z", None).await, 0, "record: no seq");
    assert_eq!(
        get(&reader, key_pair(0x52).public_key).await,
        record(0, b"z"),
        "record: read without seq"
    );
    assert_eq!(
        get(&reader, key_pair(0x53).public_key).await,
        None,
        "record: foreign key"
    );
}
