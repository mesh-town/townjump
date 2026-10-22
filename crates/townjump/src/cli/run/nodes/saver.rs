// JS: none — a port extension; holesail JS starts every node with an empty table.
// Feeds the saved nodes to the factory and writes the routing table back, periodically and at exit.
use super::{MAX_NODES, NodesOpts, load, save, unix_now};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval_at};
use townjump_dht::{Dht, DhtFactory};

/// Reads the file and hands its fresh nodes to the factory (an unusable file gives none).
pub(crate) fn seed<F: DhtFactory>(factory: &mut F, opts: &NodesOpts) {
    factory.set_nodes(load(&opts.path, unix_now(), opts.ttl));
}

/// Background writer of the table; `finish` stops it and writes once more.
pub(crate) struct Saver<D: Dht> {
    dht: Arc<D>,
    path: PathBuf,
    task: JoinHandle<()>,
}

/// Starts writing every `opts.every`, the first write one interval from now.
pub(crate) fn start<D: Dht + 'static>(dht: Arc<D>, opts: &NodesOpts) -> Saver<D> {
    let task = tokio::spawn(periodic(dht.clone(), opts.path.clone(), opts.every));
    Saver {
        dht,
        path: opts.path.clone(),
        task,
    }
}

impl<D: Dht> Saver<D> {
    /// Stops the periodic task, waits for it (two writers never overlap) and writes the last time.
    pub(crate) async fn finish(self) {
        self.task.abort();
        let _ = self.task.await;
        write_table(&*self.dht, &self.path).await;
    }
}

async fn periodic<D: Dht>(dht: Arc<D>, path: PathBuf, every: Duration) {
    let mut ticks = interval_at(Instant::now() + every, every);
    loop {
        ticks.tick().await;
        write_table(&*dht, &path).await;
    }
}

/// Empty table or an error: nothing is written (a run without network keeps the old file).
async fn write_table<D: Dht>(dht: &D, path: &std::path::Path) {
    let Ok(nodes) = dht.to_array(Some(MAX_NODES)).await else {
        return;
    };
    if nodes.is_empty() {
        return;
    }
    let _ = save(path, &nodes, unix_now());
}
