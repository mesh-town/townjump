// Server tests (general): `info` by state, `key`, a seed from the argument and a random one; server.md «The rest of the `HolesailServer` API (for the library, the CLI does not call it)».
// `start` order is in start_tests.rs, firewall in firewall_tests.rs, the record in record_tests.rs; helpers are in fixture_tests.rs.
use super::fixture::*;
use super::*;
use townjump_jsval::JsValue;
use townjump_testkit::key_pair_from_seed;
use townjump_testkit::mock::MockConfig;

mod info {
    use super::*;

    mod state {
        use super::*;

        #[tokio::test]
        async fn goes_from_none_through_listening_and_paused_to_destroyed() {
            // given: the server is started, `listen` has not finished yet
            let mut fx = setup(Setup {
                mock: MockConfig::default().hold_listen(true),
                ..Setup::default()
            })
            .await;
            fx.start().await;
            assert_eq!(fx.server.info().state, None, "before listen");
            // when: listen finished, then pause, resume and destroy
            fx.factory.release_listen();
            fx.wait_listening().await;
            fx.server.pause().await.unwrap();
            assert_eq!(fx.server.info().state, Some("paused"));
            fx.server.resume().await.unwrap();
            assert_eq!(fx.server.info().state, Some("listening"));
            fx.server.destroy().await.unwrap();
            assert_eq!(fx.server.info().state, Some("destroyed"));
        }
    }

    mod snapshot {
        use super::*;

        #[tokio::test]
        async fn private_tcp_server_has_js_fields() {
            let fx = Fixture::started(Setup::default()).await;
            fx.wait_listening().await;
            let expected = ServerInfo {
                kind: "server",
                state: Some("listening"),
                secure: true,
                port: JsValue::Number(3000.0),
                host: JsValue::String("127.0.0.1".to_string()),
                protocol: "tcp",
                seed: Some(SEED_HEX.to_string()),
                key: PRIVATE_KEY_Z32.to_string(),
                public_key: PUBLIC_KEY_Z32.to_string(),
            };
            assert_eq!(fx.server.info(), expected);
        }

        #[tokio::test]
        async fn public_udp_server_key_is_the_public_key() {
            let fx = Fixture::started(Setup {
                secure: false,
                udp: Some(JsValue::Bool(true)),
                ..Setup::default()
            })
            .await;
            let info = fx.server.info();
            assert!(!info.secure);
            assert_eq!(info.protocol, "udp");
            assert_eq!(
                (info.key.as_str(), info.public_key.as_str()),
                (PUBLIC_KEY_Z32, PUBLIC_KEY_Z32)
            );
        }
    }

    // The protocol is chosen by the truthiness of `args.udp`, not by its type.
    mod protocol {
        use super::*;

        #[tokio::test]
        async fn undefined_is_tcp() {
            assert_eq!(protocol_for(None).await, "tcp");
        }

        #[tokio::test]
        async fn false_is_tcp() {
            assert_eq!(protocol_for(Some(JsValue::Bool(false))).await, "tcp");
        }

        #[tokio::test]
        async fn zero_is_tcp() {
            assert_eq!(protocol_for(Some(JsValue::Number(0.0))).await, "tcp");
        }

        #[tokio::test]
        async fn empty_string_is_tcp() {
            assert_eq!(
                protocol_for(Some(JsValue::String(String::new()))).await,
                "tcp"
            );
        }

        #[tokio::test]
        async fn true_is_udp() {
            assert_eq!(protocol_for(Some(JsValue::Bool(true))).await, "udp");
        }

        #[tokio::test]
        async fn q6_string_false_is_udp() {
            let udp = Some(JsValue::String("false".to_string()));
            assert_eq!(protocol_for(udp).await, "udp");
        }

        #[tokio::test]
        async fn nonzero_number_is_udp() {
            assert_eq!(protocol_for(Some(JsValue::Number(1.0))).await, "udp");
        }
    }
}

mod key {
    use super::*;

    #[tokio::test]
    async fn private_mode_is_z32_of_the_seed() {
        let fx = Fixture::started(Setup::default()).await;
        assert_eq!(fx.server.key(), PRIVATE_KEY_Z32);
    }

    #[tokio::test]
    async fn public_mode_is_z32_of_the_public_key() {
        let fx = Fixture::started(Setup {
            secure: false,
            ..Setup::default()
        })
        .await;
        assert_eq!(fx.server.key(), PUBLIC_KEY_Z32);
    }

    #[tokio::test]
    async fn is_known_before_listen_completes() {
        let fx = Fixture::started(Setup {
            mock: MockConfig::default().hold_listen(true),
            ..Setup::default()
        })
        .await;
        assert_eq!(fx.server.key(), PRIVATE_KEY_Z32);
    }
}

// `seed_hex = None`: 32 random OS bytes (server.md, step 2).
mod seed {
    use super::*;

    mod absent {
        use super::*;

        #[tokio::test]
        async fn two_servers_get_different_keys() {
            let (first, second) = (random_server(true).await, random_server(true).await);
            assert_ne!(first.server.key(), second.server.key());
        }

        #[tokio::test]
        async fn private_key_is_z32_of_a_32_byte_seed_of_the_listened_pair() {
            let fx = random_server(true).await;
            let seed: [u8; 32] = z32_decode(&fx.server.key())
                .try_into()
                .expect("private mode key is not 32 bytes");
            let pair = key_pair_from_seed(&seed);
            assert_eq!(listened_key(&fx), hex::encode(pair.public_key));
        }

        #[tokio::test]
        async fn public_key_is_the_listened_ed25519_key() {
            let fx = random_server(false).await;
            let listened = hex::decode(listened_key(&fx)).expect("hex");
            assert_eq!(listened.len(), 32);
            assert_eq!(fx.server.key(), z32(&listened));
            assert_eq!(fx.server.info().public_key, z32(&listened));
        }

        // JS: `if (!seed)` - '', undefined, null and 0 give 32 random bytes (testvectors/probes-js/server-facade-dht.cjs, `generateKeyPair(...)`).
        #[tokio::test]
        async fn empty_string_is_treated_like_none() {
            let fx = Fixture::started(Setup {
                seed_hex: Some(String::new()),
                ..Setup::default()
            })
            .await;
            let seed = z32_decode(&fx.server.key());
            assert_eq!(seed.len(), 32);
            assert_ne!(seed, seed_bytes(SEED_HEX).to_vec());
        }
    }

    mod invalid {
        use super::*;

        #[tokio::test]
        async fn two_byte_seed_fails_with_the_sodium_message() {
            let (error, _) = start_error("abcd").await;
            assert!(matches!(error, ServerError::Keys(_)), "{error:?}");
            assert_eq!(
                error.to_string(),
                "seed must be 'crypto_sign_SEEDBYTES' bytes"
            );
        }

        #[tokio::test]
        async fn non_hex_seed_fails_like_an_empty_buffer() {
            let (error, _) = start_error("zz").await;
            assert_eq!(
                error.to_string(),
                "seed must be 'crypto_sign_SEEDBYTES' bytes"
            );
        }

        #[tokio::test]
        async fn bad_seed_stops_before_any_dht_call() {
            let (_, names) = start_error("abcd").await;
            assert_eq!(names, ["HyperDHT.new"]);
        }
    }
}

async fn protocol_for(udp: Option<JsValue>) -> &'static str {
    let fx = Fixture::started(Setup {
        udp,
        ..Setup::default()
    })
    .await;
    fx.server.info().protocol
}

async fn random_server(secure: bool) -> Fixture {
    Fixture::started(Setup {
        secure,
        seed_hex: None,
        ..Setup::default()
    })
    .await
}

/// Public key (hex) from the `server.listen` event.
fn listened_key(fx: &Fixture) -> String {
    let listens = fx.trace.named("server.listen");
    let event = listens.first().expect("no server.listen in the trace");
    event["publicKey"].as_str().expect("publicKey").to_string()
}

/// `start` error with seed `seed_hex` and the trace event names.
async fn start_error(seed_hex: &str) -> (ServerError, Vec<String>) {
    let mut fx = setup(Setup {
        seed_hex: Some(seed_hex.to_string()),
        ..Setup::default()
    })
    .await;
    let outcome = fx.server.start(fx.args.clone()).await;
    (outcome.expect_err("start accepted bad seed"), fx.names())
}
