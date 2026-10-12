//! The fake indexer's own write path, checked directly against the
//! admin read path (`/v1/a/ious`), independent of either dialect.

mod common;

use std::collections::BTreeMap;

use subbit_e2e::{indexer, wallet::Wallet};
use subbit_server::Keytag;

const COST: u64 = 10;

#[tokio::test]
async fn fake_indexer_backing_shows_up_with_no_iou_yet() -> anyhow::Result<()> {
    let server = common::spawn(COST).await?;
    let wallet = Wallet::new([3u8; 32], b"admin-backing".to_vec());

    indexer::push_backing(&server.base_url(), &wallet, 500, 0).await?;

    let ious = fetch_ious(&server.base_url()).await?;
    let keytag = Keytag::new(wallet.key, wallet.tag.clone());
    assert_eq!(ious.get(&keytag), Some(&None));

    server.shutdown().await?;
    Ok(())
}

async fn fetch_ious(base_url: &str) -> anyhow::Result<BTreeMap<Keytag, Option<subbit_core::Iou>>> {
    let bytes = reqwest::get(format!("{base_url}/v1/a/ious"))
        .await?
        .bytes()
        .await?;
    Ok(minicbor::decode(&bytes)?)
}
