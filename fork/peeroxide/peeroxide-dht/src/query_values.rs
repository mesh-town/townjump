// JS: dht-rpc/lib/query.js _onvisit (push(this.map(m)))
//! Value listener of a `Query`: every successful reply that carries a value is pushed to the
//! listener as it arrives, not at the query end.

use super::{IoResponseData, Query, QueryReply};

/// The `QueryReply` of one IO response (the value is cloned: the query keeps its own copy).
pub(super) fn reply_of(data: &IoResponseData) -> QueryReply {
    QueryReply {
        from: data.from.clone(),
        to: (data.to.port != 0 && !data.to.host.is_empty()).then(|| data.to.clone()),
        from_id: data.from_id,
        token: data.token,
        closer_nodes: data.closer_nodes.clone(),
        error: data.error,
        value: data.value.clone(),
        rtt: data.rtt,
    }
}

impl Query {
    /// Push a successful reply with a value to the listener; a closed channel is ignored.
    pub(super) fn emit_value(&self, data: &IoResponseData) {
        if data.error != 0 || data.value.is_none() {
            return;
        }
        if let Some(tx) = &self.values_tx {
            let _ = tx.send(reply_of(data));
        }
    }
}

#[cfg(test)]
#[path = "query_values_tests.rs"]
mod tests;
