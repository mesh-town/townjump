// Scenarios must turn red on a wrong implementation: `Broken` is a mock with one deliberately introduced defect.
use super::*;
use crate::lock::lock;
use crate::mock::{MockDht, MockFactory, MockServer};
use std::collections::HashSet;
use std::sync::Mutex;
use townjump_dht::{
    ConnectOpts, Dht, DhtConnection, DhtError, DhtFactory, Incoming, KeyPair, MutableRecord,
    ServerOpts,
};

#[derive(Clone, Copy)]
enum Bug {
    IgnoreFirewall,
    NoSuspend,
    KeepFirstRecord,
}

struct Broken(MockFactory, Bug);

struct BrokenDht {
    inner: MockDht,
    bug: Bug,
    written: Mutex<HashSet<[u8; 32]>>,
}

impl DhtFactory for Broken {
    type Dht = BrokenDht;

    async fn create(&self, identity: Option<KeyPair>) -> Result<BrokenDht, DhtError> {
        Ok(BrokenDht {
            inner: self.0.create(identity).await?,
            bug: self.1,
            written: Mutex::default(),
        })
    }
}

impl Dht for BrokenDht {
    type Server = MockServer;

    fn create_server(&self, mut opts: ServerOpts) -> Result<(MockServer, Incoming), DhtError> {
        if matches!(self.bug, Bug::IgnoreFirewall) {
            opts.firewall = None;
        }
        self.inner.create_server(opts)
    }

    async fn connect(
        &self,
        remote: [u8; 32],
        opts: ConnectOpts,
    ) -> Result<DhtConnection, DhtError> {
        self.inner.connect(remote, opts).await
    }

    async fn mutable_put(
        &self,
        kp: &KeyPair,
        value: &[u8],
        seq: Option<u64>,
    ) -> Result<u64, DhtError> {
        let again = !lock(&self.written).insert(kp.public_key);
        if again && matches!(self.bug, Bug::KeepFirstRecord) {
            return Ok(seq.unwrap_or(0));
        }
        self.inner.mutable_put(kp, value, seq).await
    }

    async fn mutable_get(
        &self,
        pk: [u8; 32],
        latest: Option<bool>,
    ) -> Result<Option<MutableRecord>, DhtError> {
        self.inner.mutable_get(pk, latest).await
    }

    async fn destroy(&self) -> Result<(), DhtError> {
        self.inner.destroy().await
    }

    async fn suspend(&self) -> Result<(), DhtError> {
        match self.bug {
            Bug::NoSuspend => Ok(()),
            _ => self.inner.suspend().await,
        }
    }

    async fn resume(&self) -> Result<(), DhtError> {
        match self.bug {
            Bug::NoSuspend => Ok(()),
            _ => self.inner.resume().await,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "foreign key: connect must have ended with an error")]
async fn firewall_scenario_fails_when_firewall_ignored() {
    firewall_foreign_key(&Broken(MockFactory::new(), Bug::IgnoreFirewall)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "while suspended: connect must have ended with an error")]
async fn suspend_scenario_fails_when_suspend_does_nothing() {
    suspend_resume(&Broken(MockFactory::new(), Bug::NoSuspend)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "record: read v2")]
async fn record_scenario_fails_when_first_record_kept() {
    mutable_record_seq(&Broken(MockFactory::new(), Bug::KeepFirstRecord)).await;
}
