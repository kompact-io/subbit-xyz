//! Drives the real `subbit_issuer::Issuer`, through its real reqwest
//! middleware, against a real spawned `subbit-server`: the thing
//! `vanilla_lifecycle.rs`'s hand-rolled wire client doesn't exercise.
//! Happy path only (deposit, spend, refund); the hand-rolled test
//! keeps the edge cases (no channel yet, exhausted channel) that
//! `Issuer`'s own stateful `next_payload` can't reproduce on demand.

mod common;

use subbit_e2e::{indexer, issuer, wallet::Wallet};

const COST: u64 = 10;
const PATH: &str = "article";
const SEED: [u8; 32] = [9u8; 32];
const TAG: &[u8] = b"issuer-vanilla-lifecycle";

#[tokio::test]
async fn deposit_spend_and_refund() -> anyhow::Result<()> {
    let server = common::spawn(COST).await?;
    let wallet = Wallet::new(SEED, TAG.to_vec());
    indexer::push_backing(&server.base_url(), &wallet, 1_000, 0).await?;

    let issuer = issuer::issuer(SEED, TAG.to_vec(), subbit_core::Duration::from_secs(60));
    // Spend costs COST, refund costs nothing: calling the latter
    // through this same middleware should never force a fresh top-up.
    let (middleware, client) = issuer::vanilla_client(issuer, |req: &reqwest::Request| {
        if req.url().path().contains("/refund/") {
            0
        } else {
            COST
        }
    });

    let base = server.base_url();

    // First spend: nothing committed yet, so the issuer pays an
    // exact-cost Iou in the same request.
    let resp = client.get(format!("{base}/v1/spend/{PATH}")).send().await?;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(middleware.cache().committed(), Some(COST));
    assert_eq!(middleware.cache().spent(), COST);

    // Refund: zero-cost through this middleware, so it goes out as a
    // liveness-only Pop, no fresh Iou. Gives the full quoted cost back.
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
