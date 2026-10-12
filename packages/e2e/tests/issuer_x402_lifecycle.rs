//! x402 mirror of `issuer_vanilla_lifecycle.rs`: the real
//! `subbit_issuer::Issuer` through its real x402 reqwest middleware,
//! against a real spawned `subbit-server`.
//!
//! The x402 middleware treats any non-success reply (including the
//! very first bare discovery request, which always comes back a
//! `402`) as a middleware error: a caller must retry once terms are
//! learned, see `subbit_issuer::reqwest::x402::Issuer`'s own doc
//! comment. That first bare call is expected to fail here.

mod common;

use subbit_e2e::{indexer, issuer, wallet::Wallet};

const COST: u64 = 10;
const PATH: &str = "article";
const SEED: [u8; 32] = [10u8; 32];
const TAG: &[u8] = b"issuer-x402-lifecycle";

#[tokio::test]
async fn deposit_discover_spend_and_refund() -> anyhow::Result<()> {
    let server = common::spawn(COST).await?;
    let wallet = Wallet::new(SEED, TAG.to_vec());
    indexer::push_backing(&server.base_url(), &wallet, 1_000, 0).await?;

    let issuer = issuer::issuer(SEED, TAG.to_vec(), subbit_core::Duration::from_secs(60));
    let (middleware, client) = issuer::x402_client(issuer, |req: &reqwest::Request| {
        if req.url().path().contains("/refund/") {
            0
        } else {
            COST
        }
    });

    let base = server.base_url();
    let spend_url = format!("{base}/v1/spend/{PATH}");

    // Nothing's been paid under yet: this goes out bare and comes
    // back a 402, which the middleware surfaces as an error while
    // caching the terms it just learned.
    assert!(client.get(&spend_url).send().await.is_err());

    // Now terms are known: this one actually pays.
    let resp = client.get(&spend_url).send().await?;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(middleware.cache().committed(), Some(COST));
    assert_eq!(middleware.cache().spent(), COST);

    // Refund: zero-cost through this middleware, so no fresh top-up.
    let resp = client
        .get(format!("{base}/v1/refund/{PATH}"))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(middleware.cache().committed(), Some(COST));
    assert_eq!(middleware.cache().spent(), 0);

    server.shutdown().await?;
    Ok(())
}
