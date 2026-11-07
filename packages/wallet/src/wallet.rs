//! `Wallet`: the surface anything that wants to build and submit a
//! transaction depends on -- address, UTXOs, signing, submission --
//! regardless of whether keys live in-process (`wallet::embedded::
//! Embedded`, this crate's only implementor) or in a browser extension
//! (a future `Cip30`, the "complement" `plan-embedded-wallet.md` names
//! but deliberately does not build here -- it's inherently wasm/JS-
//! dependent and belongs wherever it's actually wired up, most likely
//! `subbit-console`'s own wasm layer). Backend choice is a build-time
//! decision, so this stays a plain `trait` with `impl Future` returns
//! rather than paying for `dyn`-safety nobody needs.
//!
//! Ported in shape (not as a dependency) from the user's own
//! `cardano-lightning/konduit` repo, `packages/cardano/wallet/src/
//! wallet.rs` -- same `cardano-sdk` commit this workspace already pins,
//! so the types below are the exact ones already used throughout this
//! workspace's own tx-building code, not a reinterpretation.
//!
//! **One deliberate deviation from that reference**: `utxos`/`submit`
//! take `&mut self` here, not `&self`. The reference's `Wallet` is
//! shaped around CIP-30's JS calls (inherently `&self`-compatible --
//! calling a JS method never requires an exclusive Rust borrow).
//! `Embedded` instead drives I/O through `floe_client::Backend`, whose
//! own methods are `&mut self` (a short-lived, single-connection
//! client, not a shareable handle) -- forcing `Wallet::utxos`/`submit`
//! to stay `&self` here would mean giving `Embedded` an interior-
//! mutability cell purely to paper over a signature mismatch, for no
//! real benefit. `network_id`/`change_address`/`sign_tx` need no I/O
//! through a mutable backend at all (pure derivation/local signing), so
//! they stay `&self`, matching the reference exactly.

pub mod embedded;
pub mod witness;

use std::collections::BTreeMap;
use std::future::Future;

use cardano_sdk::{
    Address, Hash, Input, NetworkId, Output, Signature, Transaction, Value, VerificationKey,
    address::kind, transaction::state,
};

pub trait Wallet {
    type Error: std::error::Error + Send + Sync + 'static;

    fn network_id(&self) -> impl Future<Output = Result<NetworkId, Self::Error>>;

    /// This wallet's one address (single-address wallet, by design --
    /// see `plan-embedded-wallet.md`'s scope note). Analogous to CIP-30's
    /// `api.getChangeAddress()`.
    fn change_address(&self) -> impl Future<Output = Result<Address<kind::Any>, Self::Error>>;

    /// Analogous to CIP-30's `api.getUtxos(value, paginate)` -- pagination
    /// is not honored, but `value`, when given, is: `None` means "the
    /// wallet can't cover it."
    fn utxos(
        &mut self,
        value: Option<Value<u64>>,
    ) -> impl Future<Output = Result<Option<BTreeMap<Input, Output>>, Self::Error>>;

    /// Analogous to CIP-30's `api.signTx(tx, partialSign: true)` --
    /// always a single witness, never a complete signed transaction
    /// (the caller attaches it via `Transaction::add_witness`).
    fn sign_tx(
        &self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> impl Future<Output = Result<(VerificationKey, Signature), Self::Error>>;

    /// Analogous to CIP-30's `api.submitTx(tx: cbor<transaction>):
    /// Promise<hash32>`.
    fn submit(
        &mut self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> impl Future<Output = Result<Hash<32>, Self::Error>>;
}

/// Polls `wallet.utxos(None)` until a utxo originating from `hash`
/// appears. Exponential backoff, capped attempts: a real preprod
/// indexer lag is usually a handful of seconds, not longer. Shared by
/// every caller that submits a tx through a `Wallet` and needs to wait
/// for it to actually land before doing anything that depends on its
/// effect (`subbit-console`'s own multi-batch `submit`, `subbit-canary`'s
/// `Chain`).
///
/// `sleep` is caller-supplied rather than a direct `tokio::time::sleep`
/// call: this crate has no opinion on async runtime, and `subbit-
/// console`'s own copy of this logic has to work compiled either way
/// (native tokio, or wasm's `setTimeout`) -- see its own `sleep`
/// module's doc comment. A native-only caller (`subbit-canary`) can
/// just pass `tokio::time::sleep` directly.
///
/// A `utxos` error here is a hard failure, not treated as "not landed
/// yet": if the backend (Floe) can't answer the question right now,
/// that's a real problem for the caller to see, not something to
/// quietly paper over by assuming "no" and retrying -- a backend
/// returning a genuine error should fail loudly, not be swallowed into
/// a longer wait.
pub async fn wait_for_landing<W, S, F>(
    wallet: &mut W,
    hash: Hash<32>,
    mut sleep: S,
) -> anyhow::Result<()>
where
    W: Wallet,
    S: FnMut(std::time::Duration) -> F,
    F: Future<Output = ()>,
{
    const MAX_ATTEMPTS: u32 = 20;
    let mut wait = std::time::Duration::from_secs(2);
    for attempt in 0..MAX_ATTEMPTS {
        let utxos = wallet
            .utxos(None)
            .await
            .map_err(|e| anyhow::anyhow!("polling for {hash} to land: {e}"))?;
        if utxos.is_some_and(|utxos| utxos.keys().any(|input| input.transaction_id() == hash)) {
            return Ok(());
        }
        anyhow::ensure!(
            attempt + 1 < MAX_ATTEMPTS,
            "tx {hash} still hasn't landed after {MAX_ATTEMPTS} polls -- giving up"
        );
        sleep(wait).await;
        wait = (wait * 2).min(std::time::Duration::from_secs(30));
    }
    unreachable!("loop above always returns or bails by its last iteration")
}
