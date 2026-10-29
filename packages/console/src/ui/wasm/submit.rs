//! The one build -> sign -> aggregate witnesses -> submit path every
//! wallet backend shares, triggered by `JsConsole::submit` (renamed
//! from "checkout" -- this *is* the checkout action, just named for
//! what it does on-chain). Three named stages:
//!
//! 1. **Allocation**: `Ctx::basket_build` assembles every staged
//!    `Will`/`Open` into one transaction (`Tx::build`'s own existing
//!    design -- see `plan-subbit-console.md` section I; splitting an
//!    over-sized basket across several chained transactions is a
//!    noted, deliberately-deferred stretch goal, not built here). This
//!    function still treats the result as a collection to loop over,
//!    rather than hard-coding "exactly one tx", so that stretch goal
//!    slots in here later without this function's own shape changing.
//! 2. **Per transaction, sign + aggregate witnesses**: a transaction's
//!    `specified_signatories` (`Tx::signers`'s own dedup'd list, see
//!    `subbit_tx::step::Will::signer`) can legitimately name more than
//!    one distinct key -- e.g. two staged `Will`s on channels with
//!    different registered consumer identities. Every signatory this
//!    console's own `Keyring` already holds locally is signed with
//!    directly (`Transaction::sign`, the one-step path a raw key gets
//!    -- the same pattern `subbit-session`/`subbit-claim`'s own
//!    `keyring.rs` already use for exactly this). Whichever signatory
//!    remains uncovered is the connected submit wallet's own job
//!    (`Wallet::sign_tx` + `add_witness`, the two-step CIP-30/
//!    `Embedded` needs -- see `cip30`'s own doc on why); if its
//!    witness doesn't actually match the signatory still outstanding,
//!    that's a real "no known key for this" failure, not something to
//!    paper over by attaching it anyway.
//! 3. **Submit**: `Wallet::submit`.

use cardano_sdk::{Hash, Signature, VerificationKey};
use floe_dto_http::query::Utxo;
use subbit_wallet::Wallet;

use crate::Ctx;

pub async fn submit<W: Wallet>(
    wallet: &mut W,
    ctx: &mut Ctx,
    network: &str,
    reference_utxo: Option<Utxo>,
) -> anyhow::Result<Hash<32>> {
    let change_address = wallet.change_address().await?;
    let fuel = wallet
        .utxos(None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("wallet reported no coverable utxos"))?;

    // --- 1. allocation ---------------------------------------------
    let allocation = vec![ctx.basket_build(network, reference_utxo, change_address, &fuel)?];

    let mut hash = None;
    for mut tx in allocation {
        // --- 2. sign + aggregate witnesses ---------------------------
        let mut wallet_witness: Option<(VerificationKey, Signature)> = None;
        for vkh in tx.specified_signatories().collect::<Vec<_>>() {
            if let Some(local_key) = ctx.keyring_signing_key_for_consumer(vkh) {
                tx.sign(&local_key);
                continue;
            }
            if wallet_witness.is_none() {
                wallet_witness = Some(wallet.sign_tx(&tx).await?);
            }
            let (vkey, sig) = wallet_witness.expect("just set above if it was None");
            if Hash::<28>::new(vkey) != vkh {
                anyhow::bail!(
                    "no known key covers required signatory {vkh} (neither this console's \
                     Keyring nor the connected submit wallet)"
                );
            }
            tx.add_witness(vkey, sig);
        }

        // --- 3. submit -------------------------------------------------
        hash = Some(wallet.submit(&tx).await?);
    }
    hash.ok_or_else(|| anyhow::anyhow!("nothing staged to submit"))
}
