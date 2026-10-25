// JS: state of one mock-dht.cjs process (env, trace file), extended by an in-memory network: server registry and records.
// Related: factory.rs owns `Arc<Shared>`, dht.rs and server.rs take the trace, registry and records from it.
use crate::lock::lock;
use crate::mock::config::MockConfig;
use crate::mock::knobs::Switches;
use crate::mock::server::ServerInner;
use crate::mock::trace::MockTrace;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddrV4;
use std::sync::atomic::Ordering;
use portable_atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use townjump_dht::MutableRecord;

/// `seq` of the prepared record (like `seq: 3` in mock-dht.cjs).
const CANNED_SEQ: u64 = 3;

/// Versions of one key's record: `seq` → value.
type Versions = BTreeMap<u64, Vec<u8>>;

/// Shared factory state: two factories are two separate networks.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) config: MockConfig,
    pub(crate) trace: MockTrace,
    pub(crate) switches: Switches,
    servers: Mutex<HashMap<[u8; 32], Arc<ServerInner>>>,
    records: Mutex<HashMap<[u8; 32], Versions>>,
    anonymous: AtomicU64,
    /// Nodes passed to `set_nodes`; `None` — it was never called.
    pub(crate) seeded: Mutex<Option<Vec<SocketAddrV4>>>,
}

impl Shared {
    pub(crate) fn new(config: MockConfig) -> Self {
        let switches = Switches::new(&config);
        Shared {
            config,
            switches,
            ..Shared::default()
        }
    }

    pub(crate) fn emit(&self, event: Value) {
        self.trace.push(event);
    }

    /// A second server on the same key replaces the first.
    pub(crate) fn register(&self, pk: [u8; 32], server: Arc<ServerInner>) {
        lock(&self.servers).insert(pk, server);
    }

    /// Removes the record only if it belongs to exactly this server.
    pub(crate) fn unregister(&self, pk: &[u8; 32], server: &Arc<ServerInner>) {
        let mut servers = lock(&self.servers);
        if servers.get(pk).is_some_and(|reg| Arc::ptr_eq(reg, server)) {
            servers.remove(pk);
        }
    }

    pub(crate) fn server(&self, pk: &[u8; 32]) -> Option<Arc<ServerInner>> {
        lock(&self.servers).get(pk).cloned()
    }

    /// The same `seq` overwrites; a smaller `seq` is not rejected.
    pub(crate) fn store(&self, pk: [u8; 32], seq: u64, value: Vec<u8>) {
        lock(&self.records)
            .entry(pk)
            .or_default()
            .insert(seq, value);
    }

    /// The record with the highest `seq`.
    pub(crate) fn latest(&self, pk: &[u8; 32]) -> Option<MutableRecord> {
        let records = lock(&self.records);
        let (seq, value) = records.get(pk)?.iter().next_back()?;
        Some(MutableRecord {
            seq: *seq,
            value: value.clone(),
        })
    }

    /// The stub from the settings, otherwise the last stored record.
    pub(crate) fn find_record(&self, pk: &[u8; 32]) -> Option<MutableRecord> {
        let canned = self.config.canned(pk).map(|value| MutableRecord {
            seq: CANNED_SEQ,
            value: value.to_vec(),
        });
        canned.or_else(|| self.latest(pk))
    }

    /// Number of the next anonymous node: from 0.
    pub(crate) fn next_anonymous(&self) -> u64 {
        self.anonymous.fetch_add(1, Ordering::SeqCst)
    }
}
