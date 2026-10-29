//! End-to-end coverage of the echo demo: the real `subbit-gate`,
//! `echo-server`, `echo-proxy`, and `echo-client` binaries, wired
//! together, across both wire dialects and all three cost models.
//!
//! This is what `plan-update-echo.md` asked to demonstrate; these
//! tests are that demonstration, automated.

mod support;

use std::net::SocketAddr;

use subbit_examples_echo_client::config::{Config as ClientConfig, Dialect};
use subbit_l2_dto::{Catalog, Cost, CostByEgress, ResourceInfo, ResourceInfoOverride};
use subbit_stamp::local;
use support::{EchoProxy, EchoServer, SubbitGate};

const BACKING: u64 = 10_000;
const FLAT_COST: u64 = 5;

/// `ByRequest` on the default path, `ByEgress`/`ByClaim` overrides on
/// the other two: exactly the three resources `plan-update-echo.md`
/// asked the demo to cover.
fn demo_catalog() -> Catalog {
    Catalog {
        default: ResourceInfo {
            cost: Cost::ByRequest(FLAT_COST),
            description: "Echo demo resource".to_string(),
            mime_type: "text/plain".to_string(),
            max_timeout_seconds: 60,
            service_name: None,
            tags: None,
            icon_url: None,
        },
        resources: vec![
            (
                "/echo/egress".to_string(),
                ResourceInfoOverride {
                    cost: Some(Cost::ByEgress(CostByEgress {
                        base: 1,
                        per_kib: 1,
                        max_bytes: 65_536,
                    })),
                    ..Default::default()
                },
            ),
            (
                "/echo/claim".to_string(),
                ResourceInfoOverride {
                    cost: Some(Cost::ByClaim(50)),
                    ..Default::default()
                },
            ),
        ],
    }
}

fn gate_config(vanilla: bool, x402: bool) -> subbit_gate::Config {
    let mut config = subbit_gate::Config::default();
    config.dialects.vanilla = vanilla;
    config.dialects.x402 = x402;
    config.terms.catalog = demo_catalog();
    config
}

fn client_config(
    dialect: Dialect,
    base_url: String,
    seed: [u8; 32],
    tag: &[u8],
    cache: std::path::PathBuf,
) -> ClientConfig {
    ClientConfig {
        base_url,
        dialect,
        subbit: Some(local::Config {
            cache,
            stamp: subbit_stamp::Config::new(
                subbit_stamp::account::Config {
                    signing_key: seed,
                    tag: subbit_core::Tag::from(tag.to_vec()),
                },
                subbit_core::Duration::from_secs(3600),
            ),
            catalog: demo_catalog(),
            auto_write: true,
        }),
    }
}

/// `pubkey || tag`, the byte layout `subbit-gate`'s own `Keytag` and
/// `subbit-index`'s wire `Keytag` both use.
fn keytag_bytes(seed: [u8; 32], tag: &[u8]) -> Vec<u8> {
    let (_sk, pk) = cryptoxide::ed25519::keypair(&seed);
    let mut buf = pk.to_vec();
    buf.extend_from_slice(tag);
    buf
}

/// Posts a fake backing straight to the gate's `/v1/a/backings`, the
/// same way the real `naive-index` binary (or `mock-index` for tests)
/// would after seeing a confirmed on-chain deposit.
async fn deposit(gate_addr: SocketAddr, seed: [u8; 32], tag: &[u8]) {
    let row = subbit_index::wire::Row {
        keytag: subbit_index::wire::Keytag(keytag_bytes(seed, tag)),
        amount: Some(BACKING),
        subbed: Some(0),
    };
    let endpoint = format!("http://{gate_addr}/v1/a/backings").parse().unwrap();
    subbit_index::client::Client::new(endpoint)
        .send(vec![row])
        .await
        .expect("push backing");
}

/// Writes `config` to a fresh temp file, returning the dir (kept
/// alive by the caller) and the config's own path.
fn write_client_config(config: &ClientConfig) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("echo-client-config.toml");
    std::fs::write(&path, toml::to_string_pretty(config).unwrap()).unwrap();
    (dir, path)
}

#[tokio::test]
async fn vanilla_demonstrates_all_three_cost_models() {
    let seed = [11u8; 32];
    let tag = b"echo-demo-vanilla";

    let gate = SubbitGate::spawn(gate_config(true, false)).await;
    let echo = EchoServer::spawn().await;
    let proxy = EchoProxy::spawn(echo.addr(), gate.addr()).await;
    deposit(gate.addr(), seed, tag).await;

    let config = client_config(
        Dialect::Vanilla,
        format!("http://{}", proxy.addr()),
        seed,
        tag,
        tempfile_path("vanilla"),
    );
    let (_dir, config_path) = write_client_config(&config);

    // Same stamp, same channel, three different cost models: one
    // `echo-client` invocation per path, same as real usage (each
    // `send` is its own one-shot process).
    let flat = support::echo_client_send(&config_path, "/echo", "\"flat\"")
        .await
        .expect("flat request");
    assert_eq!(flat, "\"flat\"");

    let egress = support::echo_client_send(&config_path, "/echo/egress", "\"eg\"")
        .await
        .expect("egress request");
    assert_eq!(egress, "\"eg\"");

    let claim = support::echo_client_send(&config_path, "/echo/claim", "\"a longer claimed body\"")
        .await
        .expect("claim request");
    assert_eq!(claim, "\"a longer claimed body\"");
}

#[tokio::test]
async fn x402_demonstrates_all_three_cost_models() {
    let seed = [12u8; 32];
    let tag = b"echo-demo-x402";

    // x402 only: with both dialects configured, the first (x402,
    // see `Ctx::configured_dialects`'s own doc comment) answers the
    // fallback challenge anyway, but pinning it here keeps this test
    // from depending on that ordering.
    let gate = SubbitGate::spawn(gate_config(false, true)).await;
    let echo = EchoServer::spawn().await;
    let proxy = EchoProxy::spawn(echo.addr(), gate.addr()).await;
    deposit(gate.addr(), seed, tag).await;

    let config = client_config(
        Dialect::X402,
        format!("http://{}", proxy.addr()),
        seed,
        tag,
        tempfile_path("x402"),
    );
    let (_dir, config_path) = write_client_config(&config);

    let flat = support::echo_client_send(&config_path, "/echo", "\"flat\"")
        .await
        .expect("flat request");
    assert_eq!(flat, "\"flat\"");

    let egress = support::echo_client_send(&config_path, "/echo/egress", "\"eg\"")
        .await
        .expect("egress request");
    assert_eq!(egress, "\"eg\"");

    let claim = support::echo_client_send(&config_path, "/echo/claim", "\"a longer claimed body\"")
        .await
        .expect("claim request");
    assert_eq!(claim, "\"a longer claimed body\"");
}

fn tempfile_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "subbit-echo-demo-{label}-{}.cbor",
        std::process::id()
    ))
}
