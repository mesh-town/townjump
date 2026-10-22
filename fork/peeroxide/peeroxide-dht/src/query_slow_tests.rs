// JS: dht-rpc/lib/query.js
//! Tests for `_slow`/`oncycle` in `Query`: silent nodes, clearing slow, end of search, commit phase,
//! `node_id` in requests. Related: `query_tests.rs`.

use super::tests::{make_query_with_k, make_table, peer};
use super::*;

const TARGET: NodeId = [0u8; 32];

mod silent_nodes {
    use super::*;

    #[test]
    fn each_cycle_of_a_sent_address_frees_a_slot_for_one_more_request() {
        // given: concurrency 3, 10 nodes, 3 sent
        let mut f = setup(Opts::nodes(10));
        assert_eq!(f.sent.len(), 3, "concurrency requests sent");
        // when: the first timer expired for the three sent addresses
        let new_requests: Vec<usize> = (0..3).map(|i| count(&f.when_cycle(i), is_query)).collect();
        // then: each gave exactly one request, all three are marked slow
        assert_eq!(new_requests, [1, 1, 1]);
        assert_eq!((f.q.slow(), f.q.inflight), (3, 6));
    }

    #[test]
    fn repeated_cycle_of_the_same_address_counts_once() {
        // given: the address is already marked slow
        let mut f = setup(Opts::nodes(10));
        f.when_cycle(0);
        // when: the same address expires once more
        let again = f.when_cycle(0);
        // then: nothing new, slow as before
        assert!(again.is_empty(), "retransmission gives no requests");
        assert_eq!(f.q.slow(), 1);
    }

    #[test]
    fn address_outside_seen_is_ignored() {
        // given: a query that does not know address 10.9.9.9
        let mut f = setup(Opts::nodes(10));
        // when
        let reqs = f.q.on_cycle(&peer("10.9.9.9", 1));
        // then
        assert!(reqs.is_empty());
        assert_eq!(f.q.slow(), 0);
    }

    #[test]
    fn answered_address_is_ignored() {
        // given: node 0 replied (seen = Done)
        let mut f = setup(Opts::nodes(10));
        f.when_reply(0);
        // when: its timer expires after the response
        let reqs = f.when_cycle(0);
        // then
        assert!(reqs.is_empty());
        assert_eq!(f.q.slow(), 0);
    }

    #[test]
    fn timed_out_address_is_ignored() {
        // given: node 0 dropped by the final timeout (seen = Down)
        let mut f = setup(Opts::nodes(10));
        f.when_timeout(0);
        // when
        let reqs = f.when_cycle(0);
        // then
        assert!(reqs.is_empty());
        assert_eq!(f.q.slow(), 0);
    }
}

mod finish {
    use super::*;

    mod all_inflight_slow {
        use super::*;

        #[test]
        fn with_k_replies_finishes_and_delivers_them() {
            // given: k=2, A and B replied, C is silent
            let mut f = setup(Opts::with_k(2, 3));
            f.when_reply(0);
            f.when_reply(1);
            // when: the only inflight (C) became slow
            f.when_cycle(2);
            // then: search finished, the result with two replies was sent
            assert!(f.q.is_finished());
            let result = f.rx.try_recv().expect("result sent");
            assert_eq!(result.closest_replies.len(), 2);
        }

        #[test]
        fn with_fewer_than_k_replies_keeps_going() {
            // given: k=3, two replied, the third is silent
            let mut f = setup(Opts::with_k(3, 3));
            f.when_reply(0);
            f.when_reply(1);
            // when
            f.when_cycle(2);
            // then: fewer than k replies, the search waits
            assert!(!f.q.is_finished());
            assert!(f.rx.try_recv().is_err(), "no result yet");
        }
    }

    mod no_slow {
        use super::*;

        #[test]
        fn finishes_when_all_answered() {
            // given: k=2, two nodes
            let mut f = setup(Opts::with_k(2, 2));
            // when: both replied
            f.when_reply(0);
            f.when_reply(1);
            // then: finished, same as without slow
            assert!(f.q.is_finished());
            assert_eq!(f.rx.try_recv().expect("result").closest_replies.len(), 2);
        }

        #[test]
        fn waits_for_a_live_inflight_even_with_k_replies() {
            // given: k=2, A and B replied, C is still in flight and not slow
            let mut f = setup(Opts::with_k(2, 3));
            // when
            f.when_reply(0);
            f.when_reply(1);
            // then: a non-slow inflight keeps the search going
            assert!(!f.q.is_finished());
        }
    }
}

mod slow_released {
    use super::*;

    #[test]
    fn by_reply_from_that_node() {
        // given: node 0 is marked slow
        let mut f = setup(Opts::nodes(3));
        f.when_cycle(0);
        // when
        f.when_reply(0);
        // then
        assert_eq!(f.q.slow(), 0);
    }

    #[test]
    fn by_final_timeout_of_that_node() {
        // given: node 0 is marked slow
        let mut f = setup(Opts::nodes(3));
        f.when_cycle(0);
        // when
        f.when_timeout(0);
        // then
        assert_eq!(f.q.slow(), 0);
    }

    #[test]
    fn not_by_reply_from_another_node() {
        // given: node 0 is marked slow
        let mut f = setup(Opts::nodes(3));
        f.when_cycle(0);
        // when: node 1 replied
        f.when_reply(1);
        // then
        assert_eq!(f.q.slow(), 1);
    }
}

mod commit_phase {
    use super::*;

    #[test]
    fn slow_last_node_starts_commit_instead_of_finishing() {
        // given: commit search k=2, A and B replied with a token, C is silent
        // when: C became slow
        let (f, reqs) = commit_started();
        // then: two Commits, the query is not finished
        assert_eq!(count(&reqs, is_commit), 2);
        assert!(!f.q.is_finished());
    }

    #[test]
    fn late_events_of_the_slow_node_change_nothing() {
        // given: commit started
        let (mut f, _) = commit_started();
        // when: C expires once more and then is dropped by the final timeout
        let mut late = f.when_cycle(2);
        late.extend(f.when_timeout(2));
        // then: no new search requests, the query is not finished
        assert_eq!(count(&late, is_query), 0);
        assert!(!f.q.is_finished());
    }

    #[test]
    fn finishes_on_the_last_commit_done() {
        // given: commit started, two Commits in flight
        let (mut f, _) = commit_started();
        // when: the commit requests complete
        let first = f.q.on_commit_done();
        let second = f.q.on_commit_done();
        // then: finished only on the second
        assert!(!first && second);
        assert!(f.q.is_finished());
    }
}

mod node_id_in_requests {
    use super::*;

    #[test]
    fn search_request_names_its_node() {
        // given: three nodes via add_nodes
        // when: poll_requests sent the requests
        let f = setup(Opts::nodes(3));
        // then: node_id = peer_id of the destination
        assert!(!f.sent.is_empty());
        for p in &f.sent {
            assert_eq!(
                p.node_id,
                Some(peer_id(&p.to.host, p.to.port)),
                "to={:?}",
                p.to
            );
        }
    }

    #[test]
    fn commit_request_names_the_replying_node() {
        // given: commit started, nodes 0 and 1 replied, proving their ids
        let (_f, reqs) = commit_started();
        // when: Commit requests are collected
        let commits = params_of(&reqs, is_commit);
        // then: the request node_id is the id of the node that replied
        assert_eq!(commits.len(), 2);
        let mut ids: Vec<Option<NodeId>> = commits.iter().map(|p| p.node_id).collect();
        ids.sort();
        assert_eq!(ids, vec![Some(node_id(0)), Some(node_id(1))], "{commits:?}");
    }

    #[test]
    fn down_hint_has_none() {
        // given: A named B, B was sent
        let mut f = setup(Opts::nodes(1));
        let mut reply = response(&f.sent[0].to, Some(node_id(0)));
        reply.closer_nodes = vec![peer("127.0.0.1", 6000)];
        let after_reply = f.q.on_response(reply);
        let b = params_of(&after_reply, is_query).remove(0).to;
        // when: B dropped by the final timeout
        let reqs = f.q.on_timeout(&b);
        // then: DownHint of referrer A without node_id
        let hints = params_of(&reqs, is_down_hint);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].node_id, None);
    }
}

// ── setup and helpers ─────────────────────────────────────────────────────────

/// Preconditions: size `k`, number of added nodes, whether a commit phase is needed. Concurrency is always 3.
struct Opts {
    k: usize,
    nodes: usize,
    commit: bool,
}

impl Opts {
    /// `k = K` and an ordinary search without commit.
    fn nodes(nodes: usize) -> Self {
        Self::with_k(K, nodes)
    }

    fn with_k(k: usize, nodes: usize) -> Self {
        let commit = false;
        Self { k, nodes, commit }
    }
}

struct Fixture {
    q: Query,
    rx: oneshot::Receiver<QueryResult>,
    /// Requests returned by `poll_requests`, in send order.
    sent: Vec<RequestParams>,
}

impl Fixture {
    fn addr(&self, i: usize) -> Ipv4Peer {
        self.sent[i].to.clone()
    }

    /// The first timer of the request to the `i`-th sent node expired.
    fn when_cycle(&mut self, i: usize) -> Vec<QueryRequest> {
        let to = self.addr(i);
        self.q.on_cycle(&to)
    }

    /// The `i`-th sent node replied, proving its id.
    fn when_reply(&mut self, i: usize) -> Vec<QueryRequest> {
        self.q
            .on_response(response(&self.addr(i), Some(node_id(i))))
    }

    /// The `i`-th sent node replied with a token (for commit).
    fn when_reply_with_token(&mut self, i: usize) -> Vec<QueryRequest> {
        let mut reply = response(&self.addr(i), Some(node_id(i)));
        reply.token = Some([9u8; 32]);
        self.q.on_response(reply)
    }

    /// Final timeout of the request to the `i`-th sent node.
    fn when_timeout(&mut self, i: usize) -> Vec<QueryRequest> {
        let to = self.addr(i);
        self.q.on_timeout(&to)
    }
}

fn setup(opts: Opts) -> Fixture {
    let (mut q, rx) = if opts.commit {
        commit_query(opts.k)
    } else {
        make_query_with_k(TARGET, opts.k)
    };
    let nodes: Vec<(String, u16)> = (0..opts.nodes)
        .map(|i| ("127.0.0.1".to_string(), 5000 + i as u16))
        .collect();
    q.add_nodes(&nodes);
    let sent = params_of(&q.poll_requests(), is_query);
    Fixture { q, rx, sent }
}

/// Commit search k=2: A and B replied with a token, C is silent and expires on the first timer.
/// Returns the fixture and the requests produced by this `on_cycle(C)`.
fn commit_started() -> (Fixture, Vec<QueryRequest>) {
    let mut f = setup(Opts {
        commit: true,
        ..Opts::with_k(2, 3)
    });
    f.when_reply_with_token(0);
    f.when_reply_with_token(1);
    let reqs = f.when_cycle(2);
    (f, reqs)
}

fn commit_query(k: usize) -> (Query, oneshot::Receiver<QueryResult>) {
    let (tx, rx) = oneshot::channel();
    let table = make_table();
    let mut q = Query::new(1, TARGET, true, 2, None, 3, true, tx, table, [0u8; 32]);
    q.k = k;
    (q, rx)
}

/// The id the `i`-th node will prove in its reply; all distinct and not equal to `TARGET`.
fn node_id(i: usize) -> NodeId {
    let mut id = [0u8; 32];
    id[0] = i as u8 + 1;
    id
}

fn response(from: &Ipv4Peer, from_id: Option<NodeId>) -> IoResponseData {
    IoResponseData {
        from: from.clone(),
        sent_to: from.clone(),
        to: peer("127.0.0.1", 1),
        from_id,
        token: None,
        closer_nodes: vec![],
        error: 0,
        value: None,
        rtt: Duration::from_millis(5),
    }
}

fn is_query(r: &QueryRequest) -> bool {
    matches!(r, QueryRequest::Query(_))
}

fn is_commit(r: &QueryRequest) -> bool {
    matches!(r, QueryRequest::Commit(_))
}

fn is_down_hint(r: &QueryRequest) -> bool {
    matches!(r, QueryRequest::DownHint(_))
}

fn count(reqs: &[QueryRequest], kind: fn(&QueryRequest) -> bool) -> usize {
    reqs.iter().filter(|r| kind(r)).count()
}

/// Parameters of requests of the needed kind.
fn params_of(reqs: &[QueryRequest], kind: fn(&QueryRequest) -> bool) -> Vec<RequestParams> {
    reqs.iter()
        .filter(|r| kind(r))
        .map(|r| match r {
            QueryRequest::Query(p) | QueryRequest::Commit(p) | QueryRequest::DownHint(p) => {
                p.clone()
            }
        })
        .collect()
}
