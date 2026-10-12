//! The "fake indexer": posts `(keytag -> backing)` rows straight to a
//! running server's `/v1/a/backings`, using the exact same
//! `subbit_index::client::Client`/`wire::Row` types the real `naive`
//! indexer posts and the `mock-index` binary wraps as a CLI. No chain
//! is watched; the caller decides what "confirmed on-chain" means for
//! the scenario.

use subbit_index::client::Client;
use subbit_index::wire::{Backing, Keytag, Row};
use url::Url;

use crate::Wallet;

fn endpoint(base_url: &str) -> anyhow::Result<Url> {
    Ok(Url::parse(&format!("{base_url}/v1/a/backings"))?)
}

/// Post a backing for `wallet`'s keytag: `amount` on deposit, `subbed`
/// already spent against it (0 for a fresh deposit).
pub async fn push_backing(
    base_url: &str,
    wallet: &Wallet,
    amount: u64,
    subbed: u64,
) -> anyhow::Result<()> {
    let row = Row {
        keytag: Keytag(wallet.keytag_bytes()),
        amount: Some(amount),
        subbed: Some(subbed),
    };
    Client::new(endpoint(base_url)?).send(vec![row]).await?;
    Ok(())
}

/// Post a closed/absent backing: the indexer's way of saying a
/// channel it used to see is gone (closed, settled, or never found).
pub async fn push_closed(base_url: &str, wallet: &Wallet) -> anyhow::Result<()> {
    let row = Row {
        keytag: Keytag(wallet.keytag_bytes()),
        amount: None,
        subbed: None,
    };
    Client::new(endpoint(base_url)?).send(vec![row]).await?;
    Ok(())
}

/// Round-trip helper, mostly for assertions: what `to_backing()`
/// would have produced, for comparing against `/v1/a/ious` reads.
pub fn backing(amount: u64, subbed: u64) -> Backing {
    Backing { amount, subbed }
}
