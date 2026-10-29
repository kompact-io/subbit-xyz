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
