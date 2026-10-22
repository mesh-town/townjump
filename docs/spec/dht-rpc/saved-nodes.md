# dht-rpc saved nodes: seeding and exporting the routing table

Source: `dht-rpc/index.js` (6.27.0): constructor (`opts.nodes`), `addNode`, `_addNode`, `toArray`. Implemented in the fork (`peeroxide-dht`): `rpc/node_seeds.rs` (`seed_table`, `export`), `DhtConfig::nodes`, `DhtHandle::to_array`, `HyperDhtHandle::to_array`.

## Seeding

`opts.nodes` is an array of `{host, port}`. The constructor calls `addNode` for each entry before the bootstrap query: the node id is `peer_id(host, port)`, the entry goes into the routing table without a ping, and `added`, `seen` and `pinged` are the current tick. An entry whose id equals the table id is dropped. Fork: `DhtConfig::nodes: Vec<Ipv4Peer>`, empty by default; `DhtNode::run` seeds the table before `start_bootstrap`.

## Export

`toArray({limit})` returns `{host, port}` for the table entries, most recently seen first. `limit` 0 gives `[]`; no limit gives all. Fork: `DhtHandle::to_array(limit: Option<usize>)` (a command to the node actor, like `table_size`) and `HyperDhtHandle::to_array`. Only host and port leave the node; ids are recomputed from the address on the next start.

## Extensions in the fork (not in JS)

- Silently dropped: a host that is not a dotted IPv4 address, port 0, the node's own address (own port and a loopback, `0.0.0.0` or configured host), and repeated `host:port` (the first wins). JS would insert a bogus id or a duplicate.
- A full bucket drops the entry and discards the `BucketFull` events. JS `_onfullrow` stays silent before `bootstrapped`; the fork queues the event and would ping later.
- Bootstrap nodes stay in the join query next to the seeded ones: `start_bootstrap` takes the table plus the bootstrap list without cutting at k. JS `Query._open` skips the bootstrap nodes when the table gives at least k targets, so an old list with 20 dead entries would cut the node off the network (dht-rpc 6.27.0 with 24 dead entries does not join within 15 s; the fork joins in 2-5 s).
- A node with `nodes` but no `bootstrap` counts as joining a network: it starts ephemeral and adaptive. The fork's rule "an empty bootstrap list means the first persistent node" applies only when both are empty. In JS every node starts ephemeral.

## Differences

- The tick is 5 s; entries seeded together share tick 0 and come out in table traversal order, not insertion order (the sort is stable). The JS test "Populate DHT with options.nodes" expects insertion order.
- Dead entries are removed by the request timeout, as in JS `_ontimeout`: one retry cycle (1 s x 4). With at least k answers the query waits only for the first timer.

## Not ported

Runtime `addNode`. File input and output: the fork reads and writes no files; persistence is the caller's job.
