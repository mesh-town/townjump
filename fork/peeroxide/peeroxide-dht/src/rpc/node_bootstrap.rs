// JS: dht-rpc/index.js
//! `impl DhtNode`: bootstrap top-up for queries (js `query.js` `_open`: bootstrap nodes join the queue
//! when it holds fewer than k nodes). Neighbours: `node_query` (user queries), `node_tick` (refresh).

use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::query::{Query, parse_bootstrap_str, resolve_bootstrap_nodes};

use super::node::DhtNode;

/// Result of one name-resolution round: each raw bootstrap string with its `ip:port` form, if it resolved.
pub(super) type ResolvedBootstrap = Vec<(String, Option<String>)>;

/// Splits `raw` into resolved `ip:port` strings and the strings that did not resolve (kept for a retry).
/// Strings without a valid port can never resolve and are dropped.
pub(super) async fn split_bootstrap(raw: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut resolved, mut unresolved) = (Vec::new(), Vec::new());
    for (s, found) in resolve_each(raw).await {
        match found {
            Some(addr) => resolved.push(addr),
            None if parse_bootstrap_str(&s).is_some() => unresolved.push(s),
            None => {}
        }
    }
    (resolved, unresolved)
}

/// Upper bound for resolving ONE name; a hung `getaddrinfo` must not pin `bootstrap_resolving` forever.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

async fn resolve_each(raw: &[String]) -> ResolvedBootstrap {
    resolve_each_with(raw, RESOLVE_TIMEOUT, |s| async move {
        resolve_bootstrap_nodes(&[s]).await.pop()
    })
    .await
}

/// Resolves the names one by one; a name that takes longer than `limit` counts as unresolved.
pub(super) async fn resolve_each_with<F, Fut>(
    raw: &[String],
    limit: Duration,
    resolve: F,
) -> ResolvedBootstrap
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let found = tokio::time::timeout(limit, resolve(s.clone()))
            .await
            .ok()
            .flatten();
        out.push((s.clone(), found));
    }
    out
}

impl DhtNode {
    /// Parsed `host:port` pairs of the configured bootstrap nodes.
    pub(super) fn bootstrap_nodes(&self) -> Vec<(String, u16)> {
        self.config
            .bootstrap
            .iter()
            .filter_map(|s| parse_bootstrap_str(s))
            .collect()
    }

    /// Call right after `Query::add_from_table`: a short queue (e.g. an empty table) gets the bootstrap nodes;
    /// a full one leaves bootstrap and DNS alone (js `_open` returns before `_resolveBootstrapNodes`).
    /// Strings that failed to resolve earlier are resolved again in the background (js re-resolves per query).
    pub(super) fn top_up_from_bootstrap(&mut self, q: &mut Query) {
        if !q.is_sparse() {
            return;
        }
        q.add_nodes_if_sparse(&self.bootstrap_nodes());
        self.retry_unresolved_bootstrap();
    }

    /// At most one resolution runs at a time; the actor loop never waits for DNS.
    fn retry_unresolved_bootstrap(&mut self) {
        if self.bootstrap_unresolved.is_empty() || self.bootstrap_resolving {
            return;
        }
        self.bootstrap_resolving = true;
        let raw = self.bootstrap_unresolved.clone();
        let tx: mpsc::UnboundedSender<ResolvedBootstrap> = self.bootstrap_resolved_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(resolve_each(&raw).await);
        });
    }

    /// Resolved addresses join `config.bootstrap`; if the table is empty the node asks them right away.
    pub(super) fn handle_bootstrap_resolved(&mut self, done: ResolvedBootstrap) {
        self.bootstrap_resolving = false;
        let mut any = false;
        for (raw, found) in done {
            let Some(addr) = found else { continue };
            self.bootstrap_unresolved.retain(|s| *s != raw);
            if !self.config.bootstrap.contains(&addr) {
                self.config.bootstrap.push(addr);
            }
            any = true;
        }
        let empty = self.table.lock().map(|t| t.is_empty()).unwrap_or(false);
        if any && empty && self.bootstrapped && !self.is_suspended() {
            self.run_refresh();
        }
    }
}

#[cfg(test)]
#[path = "node_bootstrap_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "node_bootstrap_resolve_tests.rs"]
mod resolve_tests;
