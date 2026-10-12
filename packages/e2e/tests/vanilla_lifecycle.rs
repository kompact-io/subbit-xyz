//! Vanilla's side of the same lifecycle `x402_lifecycle.rs` covers:
//! no channel yet, a fake-indexer deposit, an Iou establishing a
//! committed amount, a liveness-only spend, and a refund.

mod common;

use subbit_e2e::{indexer, vanilla::VanillaClient, wallet::Wallet};
use subbit_l2_dto::{CoreError, Request};

const COST: u64 = 10;
const PATH: &str = "/article";

fn pop(wallet: &Wallet) -> Request {
    wallet.pop_payload(subbit_core::Duration::from_secs(60))
}

#[tokio::test]
async fn deposit_pay_and_refund() -> anyhow::Result<()> {
    let server = common::spawn(COST).await?;
    let client = VanillaClient::new(server.addr)?;
    let wallet = Wallet::new([1u8; 32], b"vanilla-lifecycle".to_vec());

    // Before the fake indexer has posted anything, there's no channel
    // at all.
    let resp = client.spend(PATH, &pop(&wallet)).await?;
    assert_eq!(resp.error, Some(CoreError::NoChannel));

    // The fake indexer populates the db with a backing: the channel
    // now exists, but nothing's committed to it yet, so it still
    // can't be spent against.
    indexer::push_backing(&server.base_url(), &wallet, 1_000, 0).await?;
    let resp = client.spend(PATH, &pop(&wallet)).await?;
    assert_eq!(resp.error, Some(CoreError::Unspendable));

    // An Iou establishes the committed amount and pays `COST` in the
    // same request.
    let iou = wallet.iou_payload(50);
    let resp = client.spend(PATH, &iou).await?;
    assert_eq!(resp.status.channel.unwrap().spendable, Some(40));

    // A liveness-only follow-up spends `COST` again with no new Iou,
    // and gets a session Mac back (vanilla's own convergence with
    // x402's `PopMac` path, see `plan-dialects.md`).
    let resp = client.spend(PATH, &pop(&wallet)).await?;
    assert_eq!(resp.status.channel.unwrap().spendable, Some(30));
    assert!(resp.status.mac.is_some());

    // Refund gives `COST` back off the running spent total.
    let resp = client.refund(PATH, &pop(&wallet)).await?;
    assert_eq!(resp.status.channel.unwrap().spendable, Some(40));

    server.shutdown().await?;
    Ok(())
}
