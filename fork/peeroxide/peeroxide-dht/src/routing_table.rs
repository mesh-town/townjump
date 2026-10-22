use rand::Rng;

use crate::peer::NodeId;

/// K parameter — max nodes per bucket
pub const K: usize = 20;

/// Number of buckets for 32-byte IDs
const NUM_BUCKETS: usize = 256;

/// A peer in the routing table
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub host: String,
    pub port: u16,
    pub token: Option<Vec<u8>>,
    pub added_tick: u64,
    pub seen_tick: u64,
    pub pinged_tick: u64,
    pub down_hints: u32,
}

/// Events emitted by the routing table
#[derive(Debug)]
pub enum TableEvent {
    /// A bucket reached capacity and rejected a node — ping oldest to decide eviction
    BucketFull { new_node: Node, bucket_index: usize },
}

/// A single k-bucket, nodes sorted by ID for binary search
struct Bucket {
    nodes: Vec<Node>,
}

impl Bucket {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }
}

/// The Kademlia routing table
pub struct RoutingTable {
    id: NodeId,
    k: usize,
    buckets: Vec<Option<Bucket>>,
    size: usize,
    pending_events: Vec<TableEvent>,
}

impl RoutingTable {
    /// Create a new routing table with the given local node ID
    pub fn new(id: NodeId) -> Self {
        Self::with_k(id, K)
    }

    /// Create with custom k value
    pub fn with_k(id: NodeId, k: usize) -> Self {
        let buckets = (0..NUM_BUCKETS).map(|_| None).collect();
        Self {
            id,
            k,
            buckets,
            size: 0,
            pending_events: Vec::new(),
        }
    }

    /// Get the local node ID
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// Get total number of nodes
    pub fn len(&self) -> usize {
        self.size
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Add a node. Returns true if added/replaced, false if bucket full.
    /// If bucket full, pushes a `TableEvent::BucketFull` event.
    pub fn add(&mut self, node: Node) -> bool {
        let idx = self.bucket_index(&node.id);

        if self.buckets[idx].is_none() {
            self.buckets[idx] = Some(Bucket::new());
        }

        // Two-phase access: compute position with an immutable borrow, then
        // mutate in a separate scope so the borrow checker sees no overlap.
        let search = {
            let bucket = match self.buckets[idx].as_ref() {
                Some(b) => b,
                None => return false,
            };
            match bucket.nodes.binary_search_by(|n| n.id.cmp(&node.id)) {
                Ok(pos) => Ok(pos),
                Err(pos) => Err((pos, bucket.nodes.len())),
            }
        };

        match search {
            Ok(pos) => {
                if let Some(bucket) = self.buckets[idx].as_mut() {
                    bucket.nodes[pos] = node;
                }
                true
            }
            Err((pos, len)) if len < self.k => {
                if let Some(bucket) = self.buckets[idx].as_mut() {
                    bucket.nodes.insert(pos, node);
                }
                self.size += 1;
                true
            }
            Err(_) => {
                self.pending_events.push(TableEvent::BucketFull {
                    new_node: node,
                    bucket_index: idx,
                });
                false
            }
        }
    }

    /// All entries, bucket by bucket.
    pub fn iter(&self) -> impl Iterator<Item = &Node> + '_ {
        self.buckets
            .iter()
            .filter_map(|b| b.as_ref())
            .flat_map(|b| b.nodes.iter())
    }

    /// Remove a node by ID. Returns the removed node if found.
    pub fn remove(&mut self, id: &NodeId) -> Option<Node> {
        let idx = self.bucket_index(id);

        let pos = {
            let bucket = self.buckets[idx].as_ref()?;
            bucket.nodes.binary_search_by(|n| n.id.cmp(id)).ok()?
        };

        let node = {
            let bucket = self.buckets[idx].as_mut()?;
            bucket.nodes.remove(pos)
        };

        self.size -= 1;
        Some(node)
    }

    /// Check if a node with this ID exists
    pub fn has(&self, id: &NodeId) -> bool {
        self.get(id).is_some()
    }

    /// Get a reference to a node by ID
    pub fn get(&self, id: &NodeId) -> Option<&Node> {
        let idx = self.bucket_index(id);
        let bucket = self.buckets[idx].as_ref()?;
        let pos = bucket.nodes.binary_search_by(|n| n.id.cmp(id)).ok()?;
        Some(&bucket.nodes[pos])
    }

    /// Get a mutable reference to a node by ID
    pub fn get_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        let idx = self.bucket_index(id);

        let pos = {
            let bucket = self.buckets[idx].as_ref()?;
            bucket.nodes.binary_search_by(|n| n.id.cmp(id)).ok()?
        };

        let bucket = self.buckets[idx].as_mut()?;
        Some(&mut bucket.nodes[pos])
    }

    /// Find up to `k` closest nodes to `target`.
    ///
    /// Ported from kademlia-routing-table (Node.js v1.0.6):
    /// 1. Start at the bucket for `target` (index `d`).
    /// 2. Scan downward (d → 0) collecting nodes up to k.
    /// 3. If still under k, scan upward (d+1 → NUM_BUCKETS-1).
    pub fn closest(&self, target: &NodeId, k: usize) -> Vec<&Node> {
        let d = self.bucket_index(target);
        let mut result = Vec::with_capacity(k);

        let mut i = d as isize;
        while i >= 0 {
            if result.len() >= k {
                break;
            }
            if let Some(bucket) = &self.buckets[i as usize] {
                for node in &bucket.nodes {
                    if result.len() >= k {
                        break;
                    }
                    result.push(node);
                }
            }
            i -= 1;
        }

        let mut j = d + 1;
        while j < NUM_BUCKETS {
            if result.len() >= k {
                break;
            }
            if let Some(bucket) = &self.buckets[j] {
                for node in &bucket.nodes {
                    if result.len() >= k {
                        break;
                    }
                    result.push(node);
                }
            }
            j += 1;
        }

        result
    }

    /// Rebuild the table with a new local node ID.
    ///
    /// Extracts all nodes, resets buckets with the new ID, and re-inserts
    /// each node. Nodes that no longer fit (due to bucket capacity under the
    /// new distance metric) are dropped. Mirrors the Node.js `_updateNetworkState`
    /// table rebuild in dht-rpc.
    pub fn rebuild_with_id(&mut self, new_id: NodeId) {
        if self.id == new_id {
            return;
        }
        let nodes: Vec<Node> = self
            .buckets
            .iter_mut()
            .filter_map(|b| b.as_mut())
            .flat_map(|b| b.nodes.drain(..))
            .collect();

        self.id = new_id;
        self.size = 0;
        for bucket in self.buckets.iter_mut() {
            *bucket = None;
        }
        self.pending_events.clear();

        for node in nodes {
            if node.id == new_id {
                continue;
            }
            self.add(node);
        }
    }

    /// Return a random node from the table, or `None` if empty.
    pub fn random(&self) -> Option<&Node> {
        if self.size == 0 {
            return None;
        }

        let occupied: Vec<usize> = self
            .buckets
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b {
                Some(bucket) if !bucket.nodes.is_empty() => Some(i),
                _ => None,
            })
            .collect();

        if occupied.is_empty() {
            return None;
        }

        let mut rng = rand::rng();
        let bucket_idx = occupied[rng.random_range(0..occupied.len())];
        let bucket = self.buckets[bucket_idx].as_ref()?;
        let node_idx = rng.random_range(0..bucket.nodes.len());
        Some(&bucket.nodes[node_idx])
    }

    /// Return up to `limit` nodes ordered by `seen_tick` descending (most
    /// recently seen first).
    ///
    /// When the table holds 8 or more nodes the 5 most-recently-seen are
    /// skipped; they are likely nodes engaged in current traffic and should
    /// not be re-used as NAT-sample ping targets.  Mirrors the candidate
    /// selection in `lib/nat.js:31-59` of the Node.js reference.
    #[allow(dead_code)]
    pub(crate) fn recent(&self, limit: usize) -> Vec<&Node> {
        if limit == 0 {
            return Vec::new();
        }
        let mut all: Vec<&Node> = self
            .buckets
            .iter()
            .filter_map(|b| b.as_ref())
            .flat_map(|b| b.nodes.iter())
            .collect();
        all.sort_by_key(|n| std::cmp::Reverse(n.seen_tick));
        let skip = if all.len() >= 8 { 5 } else { 0 };
        all.into_iter().skip(skip).take(limit).collect()
    }

    /// Drain and return all pending events.
    pub fn drain_events(&mut self) -> Vec<TableEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Compute the bucket index for `id` using XOR distance from local ID.
    ///
    /// Ported from kademlia-routing-table JS `_diff()`:
    /// ```js
    /// for (let i = 0; i < id.length; i++) {
    ///   const a = id[i], b = this.id[i];
    ///   if (a !== b) return i * 8 + Math.clz32(a ^ b) - 24;
    /// }
    /// return this.rows.length - 1;
    /// ```
    /// `Math.clz32(byte) - 24` = `u8::leading_zeros()` since clz32 on a byte
    /// always starts with 24 extra leading zeros in the 32-bit representation.
    fn bucket_index(&self, id: &NodeId) -> usize {
        for (i, (&a, &b)) in id.iter().zip(self.id.iter()).enumerate() {
            if a != b {
                return i * 8 + (a ^ b).leading_zeros() as usize;
            }
        }
        NUM_BUCKETS - 1
    }
}

#[cfg(test)]
#[path = "routing_table_tests.rs"]
mod tests;
