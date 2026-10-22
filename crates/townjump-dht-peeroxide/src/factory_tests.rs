// Tests of the `factory` module: `from_env_value` (the pure part of reading `TOWNJUMP_BOOTSTRAP`) and `create` rejecting a malformed bootstrap, no network.
// `set_var` is unavailable under `forbid(unsafe_code)`, so the environment variable arrives here as a value, not through the process.
use super::*;
use crate::bootstrap::BootstrapPlan;
use std::net::SocketAddrV4;
use std::time::Duration;

const CREATE_WAIT: Duration = Duration::from_secs(30);

mod from_env_value {
    use super::*;

    mod variable_unset {
        use super::*;

        #[test]
        fn uses_the_default_bootstrap() {
            // given: no variable; when: from_env; then: `None` — the fork default
            assert_reads(None, None);
        }
    }

    mod variable_empty {
        use super::*;

        #[test]
        fn uses_the_default_bootstrap() {
            assert_reads(Some(""), None);
        }
    }

    mod one_address {
        use super::*;

        #[test]
        fn is_kept_as_host_and_port() {
            assert_reads(Some("10.1.2.3:49737"), Some(&["10.1.2.3:49737"]));
        }
    }

    mod several_addresses {
        use super::*;

        #[test]
        fn keep_their_order() {
            let value = "10.0.0.1:1,10.0.0.2:2,10.0.0.3:3";
            let expected = ["10.0.0.1:1", "10.0.0.2:2", "10.0.0.3:3"];
            assert_reads(Some(value), Some(&expected));
        }
    }

    mod garbage_value {
        use super::*;

        // A malformed value does not break `from_env_value`, the error comes later, from `create`.
        #[test]
        fn is_kept_for_create_to_refuse() {
            assert_reads(Some("nonsense"), Some(&["nonsense"]));
        }
    }
}

mod adaptive_var {
    use super::*;

    #[test]
    fn unset_empty_and_other_values_leave_it_on() {
        for value in [None, Some("1"), Some("yes"), Some(""), Some("true")] {
            assert!(adaptive_from_env(value), "{value:?}");
        }
    }

    #[test]
    fn off_words_switch_it_off_in_any_case() {
        for value in ["0", "false", "OFF", " no ", "No"] {
            assert!(!adaptive_from_env(Some(value)), "{value:?}");
        }
    }
}

mod factory_defaults {
    use super::*;

    #[test]
    fn adaptive_is_on() {
        assert!(PeeroxideFactory::default().adaptive);
    }

    #[test]
    fn from_env_with_reads_the_adaptive_variable_only() {
        let got =
            PeeroxideFactory::from_env_with(|k| (k == "TOWNJUMP_DHT_ADAPTIVE").then(|| "0".into()));
        assert!(!got.adaptive);
        assert_eq!(got.bootstrap, None);
    }

    #[test]
    fn from_env_with_reads_both_variables() {
        let got = PeeroxideFactory::from_env_with(|k| match k {
            "TOWNJUMP_BOOTSTRAP" => Some("10.0.0.1:1".to_string()),
            _ => None,
        });
        assert!(got.adaptive);
        assert_eq!(got.bootstrap, Some(vec!["10.0.0.1:1".to_string()]));
    }
}

mod node_config {
    use super::*;

    fn nodes_of(list: &[&str]) -> Vec<SocketAddrV4> {
        list.iter().map(|s| s.parse().expect("addr")).collect()
    }

    fn peers_of(factory: &PeeroxideFactory) -> Vec<(String, u16)> {
        let plan = BootstrapPlan::parse(Some(&["10.0.0.1:1".to_string()])).expect("plan");
        let config = factory.node_config(&plan);
        config
            .dht
            .nodes
            .into_iter()
            .map(|p| (p.host, p.port))
            .collect()
    }

    #[test]
    fn default_has_no_nodes() {
        let factory = PeeroxideFactory::default();
        assert!(factory.nodes.is_empty());
        assert!(peers_of(&factory).is_empty());
    }

    #[test]
    fn passes_nodes_to_the_dht_config() {
        let mut factory = PeeroxideFactory::default();
        factory.set_nodes(nodes_of(&["1.2.3.4:5", "6.7.8.9:10"]));
        let want = vec![("1.2.3.4".to_string(), 5), ("6.7.8.9".to_string(), 10)];
        assert_eq!(peers_of(&factory), want);
    }

    #[test]
    fn set_nodes_replaces() {
        let mut factory = PeeroxideFactory::default();
        factory.set_nodes(nodes_of(&["1.2.3.4:5"]));
        factory.set_nodes(nodes_of(&["6.7.8.9:10"]));
        assert_eq!(factory.nodes, nodes_of(&["6.7.8.9:10"]));
    }

    #[test]
    fn from_env_has_no_nodes() {
        assert!(PeeroxideFactory::from_env_with(|_| None).nodes.is_empty());
    }

    fn config_of(adaptive: bool, bootstrap: &[&str]) -> HyperDhtConfig {
        let list: Vec<String> = bootstrap.iter().map(|s| s.to_string()).collect();
        let plan = BootstrapPlan::parse(Some(&list)).expect("plan");
        let factory = PeeroxideFactory {
            bootstrap: None,
            adaptive,
            ..Default::default()
        };
        factory.node_config(&plan)
    }

    #[test]
    fn passes_adaptive_to_the_dht_config() {
        assert!(config_of(true, &["10.0.0.1:1"]).dht.adaptive);
        assert!(!config_of(false, &["10.0.0.1:1"]).dht.adaptive);
    }

    #[test]
    fn keeps_ephemeral_as_the_plan_sets_it() {
        assert_eq!(config_of(true, &["10.0.0.1:1"]).dht.ephemeral, None);
        assert_eq!(config_of(true, &["127.0.0.1:1"]).dht.ephemeral, Some(true));
    }
}

mod create_with_garbage_bootstrap {
    use super::*;

    mod address_without_port {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn is_refused() {
            assert_create_refused(&["nonsense"]).await;
        }
    }

    mod port_is_not_a_number {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn is_refused() {
            assert_create_refused(&["127.0.0.1:port"]).await;
        }
    }

    mod port_is_out_of_range {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn is_refused() {
            assert_create_refused(&["127.0.0.1:70000"]).await;
        }
    }

    mod one_bad_address_among_good_ones {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn is_refused() {
            assert_create_refused(&["127.0.0.1:49737", "nonsense"]).await;
        }
    }
}

/// `from_env_value(value).bootstrap` equals `expected` (`None` — the fork default).
fn assert_reads(value: Option<&str>, expected: Option<&[&str]>) {
    let got = PeeroxideFactory::from_env_value(value).bootstrap;
    let expected: Option<Vec<String>> =
        expected.map(|list| list.iter().map(|s| s.to_string()).collect());
    assert_eq!(got, expected, "from_env_value({value:?})");
}

/// `create` with this bootstrap ends in an error (no network and not later than `CREATE_WAIT`).
async fn assert_create_refused(bootstrap: &[&str]) {
    let factory = PeeroxideFactory {
        bootstrap: Some(bootstrap.iter().map(|s| s.to_string()).collect()),
        ..Default::default()
    };
    let created = tokio::time::timeout(CREATE_WAIT, factory.create(None)).await;
    let created = created.unwrap_or_else(|_| panic!("create {bootstrap:?} hung"));
    assert!(
        created.is_err(),
        "create accepted a malformed bootstrap {bootstrap:?}"
    );
}
