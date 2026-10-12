//! Mirrors `tmp-subbit-x402`'s `spike/x402/e2e.ts` lifecycle, minus
//! any real chain: challenge, a fake-indexer deposit, paying vouchers,
//! exhausting the channel (the corrective-402 path), and a top-up
//! recovering it.

mod common;

use subbit_e2e::{indexer, wallet::Wallet, x402::Reply, x402::X402Client};
use subbit_l2_dto::dialect::x402::{KnownExtra, Require, SubbitAccept, SubbitExtra, extension};
use subbit_l2_dto::{Commit, Request};

const COST: u64 = 10;
const PATH: &str = "/article";

fn pop(wallet: &Wallet) -> Request {
    wallet.pop_payload(subbit_core::Duration::from_secs(60))
}

fn accepted_of(required: &Require) -> SubbitAccept {
    required.accepts[0].clone()
}

#[tokio::test]
async fn deposit_pay_exhaust_and_topup() -> anyhow::Result<()> {
    let server = common::spawn(COST).await?;
    let client = X402Client::new(server.base_url());
    let wallet = Wallet::new([7u8; 32], b"x402-lifecycle".to_vec());

    // 1. No PAYMENT-SIGNATURE yet: the initial challenge, advertising
    //    this route's cost.
    let (status, required) = client.challenge(PATH).await?;
    assert_eq!(status, reqwest::StatusCode::PAYMENT_REQUIRED);
    assert_eq!(required.accepts[0].amount, COST.to_string());
    let accepted = accepted_of(&required);

    // 2. The fake indexer confirms a deposit, off-chain, no real chain
    //    involved at all.
    indexer::push_backing(&server.base_url(), &wallet, 1_000, 0).await?;

    // 3. The first Iou establishes the committed amount; this
    //    request's cost is deducted in the same call.
    let (status, reply) = client
        .pay(PATH, accepted.clone(), wallet.iou_payload(50))
        .await?;
    assert_eq!(status, reqwest::StatusCode::OK);
    let Reply::Settlement(settlement) = reply else {
        panic!("expected a settlement");
    };
    assert!(settlement.success);
    assert_eq!(
        extension(&settlement).unwrap().channel.unwrap().spendable,
        Some(40)
    );

    // 4. Liveness-only requests (no new amount) keep deducting cost,
    //    exhausting spendable down to zero.
    for expected in [30u64, 20, 10, 0] {
        let (status, reply) = client.pay(PATH, accepted.clone(), pop(&wallet)).await?;
        assert_eq!(status, reqwest::StatusCode::OK);
        let Reply::Settlement(settlement) = reply else {
            panic!("expected a settlement");
        };
        assert_eq!(
            extension(&settlement).unwrap().channel.unwrap().spendable,
            Some(expected)
        );
    }

    // 5. One more request can't be spent: a corrective 402 reporting
    //    the channel's real status, not a bare error, the client can
    //    size its next top-up off this.
    let (status, reply) = client.pay(PATH, accepted.clone(), pop(&wallet)).await?;
    assert_eq!(status, reqwest::StatusCode::PAYMENT_REQUIRED);
    let Reply::Required(corrective) = reply else {
        panic!("expected a corrective challenge");
    };
    match &corrective.accepts[0].extra {
        SubbitExtra::Known(KnownExtra::SubbitV1 { channel_status, .. }) => {
            assert_eq!(channel_status.as_ref().unwrap().spendable, Some(0));
        }
        SubbitExtra::Unknown(_) => panic!("expected a standard extra"),
    }

    // 6. A top-up Iou recovers spendable.
    let (status, reply) = client.pay(PATH, accepted, wallet.iou_payload(100)).await?;
    assert_eq!(status, reqwest::StatusCode::OK);
    let Reply::Settlement(settlement) = reply else {
        panic!("expected a settlement");
    };
    assert_eq!(
        extension(&settlement).unwrap().channel.unwrap().spendable,
        Some(40)
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn mutual_close_is_not_yet_supported() -> anyhow::Result<()> {
    // `Ctx` always runs with `mutual::Unsupported` today, a
    // regression anchor for when a real `MutualService` lands.
    let server = common::spawn(COST).await?;
    let client = X402Client::new(server.base_url());
    let wallet = Wallet::new([8u8; 32], b"x402-mutual".to_vec());
    indexer::push_backing(&server.base_url(), &wallet, 1_000, 0).await?;

    let (_, required) = client.challenge(PATH).await?;
    let accepted = accepted_of(&required);

    let payload = Request::ById {
        channel_id: wallet.channel_id(),
        commit: Commit::Mutual(subbit_l2_dto::Mutual {
            transaction: vec![0xde, 0xad],
        }),
    };
    let (status, reply) = client.pay(PATH, accepted, payload).await?;
    assert_eq!(status, reqwest::StatusCode::PAYMENT_REQUIRED);
    let Reply::Settlement(settlement) = reply else {
        panic!("expected a settlement");
    };
    assert!(!settlement.success);
    assert!(
        settlement
            .error_reason
            .unwrap()
            .to_lowercase()
            .contains("mutual")
    );

    server.shutdown().await?;
    Ok(())
}
