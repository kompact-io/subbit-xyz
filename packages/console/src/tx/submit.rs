//! The one build -> sign -> aggregate witnesses -> submit path every
//! wallet backend shares -- part of the core logic engine (`lib.rs`'s
//! own doc), not a UI concern: `ui::wasm::JsConsole::submit` and any
//! native caller (`ui::cli`, `subbit-canary`) both call this directly,
//! rather than either reimplementing it. Three named stages:
//!
//! 1. **Allocation**: `Ctx::basket_build_batch` assembles every staged
//!    `Will` plus up to `Basket::MAX_OPENS_PER_TX` staged `Open`s into
//!    one transaction (`Tx::build`'s own existing design -- see
//!    `plan-subbit-console.md` section I). A basket with more Opens
//!    than that drains across as many sequentially-submitted
//!    transactions as it takes -- each loop iteration re-fetches `fuel`
//!    and, when a previous iteration already landed one batch and more
//!    remain, first waits for it (`wait_for_landing`) so that re-fetch
//!    doesn't just see the same now-stale utxo again. `Will`s aren't
//!    capped the same way (see `Basket::build_batch`'s own doc on why),
//!    so they're always fully drained by the first iteration.
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
//!
//! **Retry/backoff**: ported in shape (not as a dependency) from
//! `subbit-canary`'s own `l1.rs` (`back_off`/`missing_input`/
//! `too_early_wait`, measured against real preprod rejections, before
//! canary moved onto this shared engine). On a submit rejection:
//! - a `MaxTxSizeUTxO`-shaped rejection shrinks the *Opens* side of the
//!   next attempt (`back_off`, scaling by the rejection's own reported
//!   `supplied`/`expected` byte counts) -- safe because `build_batch`
//!   can always stash the excess back out and retry smaller.
//! - a "missing transaction input" rejection naming one of this
//!   attempt's own staged `Will`s means something else (most likely a
//!   concurrently-running real `subbit-claim` process) already moved
//!   that channel on; whatever this `Will` was trying to do is moot,
//!   so it's dropped (`basket_drop_intent`) and the rest of the batch
//!   retries without it.
//! - a "not within validity interval" rejection waits out the reported
//!   shortfall (+5s buffer) and retries the same attempt unchanged.
//!
//! Every attempt snapshots `Ctx` first and restores it on rejection:
//! `Basket::build`'s own "unchanged on failure" guarantee only covers
//! *build* failures, not a *submit* failure after a successful build
//! (`ctx.basket_build_batch` already consumed the attempted Wills/Opens
//! by the time `wallet.submit` runs) -- without the restore, a
//! rejected submit would silently lose whatever it had just staged.
//! `Will`s themselves still aren't batched/shrunk by count the way
//! `Open`s are (same reason `build_batch` doesn't: a staged `Will`
//! doesn't retain the original `Want`, so there's nothing to safely
//! restage if a large batch of *Wills* alone -- no Opens to shrink --
//! is what a `MaxTxSizeUTxO` rejection comes back for). That's a real,
//! known gap, not an oversight: revisit if/when it's actually hit.

use std::time::Duration;

use cardano_sdk::{Hash, Signature, VerificationKey};
use subbit_wallet::{Wallet, wait_for_landing};

use super::Basket;
use crate::{Ctx, Error};

#[tracing::instrument(skip(wallet, ctx), ret(Debug), err(Debug))]
pub async fn submit<W: Wallet>(
    wallet: &mut W,
    ctx: &mut Ctx,
    network: &str,
) -> Result<Hash<32>, Error> {
    let change_address = wallet
        .change_address()
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;

    if ctx.basket_opens().is_empty() && ctx.basket_wills().is_empty() {
        return Err(Error::Basket("nothing staged to submit".to_string()));
    }

    let mut max_opens = Basket::MAX_OPENS_PER_TX;
    let mut too_early_retries = 0u32;

    let hash = loop {
        // Re-fetched every iteration: a previous iteration's submit may
        // have spent the very utxo a stale `fuel` would otherwise try
        // to reuse.
        let fuel = wallet
            .utxos(None)
            .await
            .map_err(|e| Error::Wallet(e.to_string()))?
            .ok_or_else(|| Error::Wallet("wallet reported no coverable utxos".to_string()))?;

        // Snapshotted before build: `basket_build_batch` consumes the
        // attempted Wills/Opens on success, before `wallet.submit` ever
        // runs -- restored below on any rejection, so a failed submit
        // never silently loses what it had staged.
        let snapshot = ctx.clone();

        // --- 1. allocation ---------------------------------------------
        let attempted_opens = ctx.basket_opens().len().min(max_opens);
        let attempted_wills = ctx.basket_wills().len();
        let mut tx = ctx.basket_build_batch(network, change_address.clone(), &fuel, max_opens)?;

        // --- 2. sign + aggregate witnesses -------------------------------
        let mut wallet_witness: Option<(VerificationKey, Signature)> = None;
        let mut sign_failure = None;
        for vkh in tx.specified_signatories().collect::<Vec<_>>() {
            if let Some(local_key) = ctx.keyring_signing_key_for_consumer(vkh) {
                tx.sign(&local_key);
                continue;
            }
            if wallet_witness.is_none() {
                wallet_witness = Some(
                    wallet
                        .sign_tx(&tx)
                        .await
                        .map_err(|e| Error::Wallet(e.to_string()))?,
                );
            }
            let (vkey, sig) = wallet_witness.expect("just set above if it was None");
            if Hash::<28>::new(vkey) != vkh {
                sign_failure = Some(Error::Wallet(format!(
                    "no known key covers required signatory {vkh} (neither this console's \
                     Keyring nor the connected submit wallet)"
                )));
                break;
            }
            tx.add_witness(vkey, sig);
        }
        if let Some(e) = sign_failure {
            *ctx = snapshot;
            return Err(e);
        }

        // The connected wallet's own witness, for the fuel inputs *it*
        // contributed -- unconditional, not just the loop's own
        // fallback for an uncovered `specified_signatories` entry:
        // ordinary input-spending authorization is a separate ledger
        // requirement from a validator's own required signers, and
        // nothing above adds it unless the wallet's own vkh happened to
        // also be a specified signatory. `add_witness` is idempotent
        // for an already-added vkey, so this is safe to call even when
        // the loop above already added it.
        let (vkey, sig) = match wallet_witness {
            Some(witness) => witness,
            None => wallet
                .sign_tx(&tx)
                .await
                .map_err(|e| Error::Wallet(e.to_string()))?,
        };
        tx.add_witness(vkey, sig);

        // --- 3. submit ---------------------------------------------------
        match wallet.submit(&tx).await {
            Ok(hash) => {
                tracing::info!(
                    %hash,
                    opens = attempted_opens,
                    wills = attempted_wills,
                    "batch landed"
                );
                max_opens = Basket::MAX_OPENS_PER_TX;
                too_early_retries = 0;
                // Always wait for confirmation before returning, not
                // just when more Opens batches remain: a caller that
                // immediately re-reads chain state afterward (e.g.
                // `subbit-canary`'s own per-round `refresh`, deciding
                // the next round's `Want` off what `Can` now allows)
                // would otherwise race this submission's own
                // not-yet-confirmed effect, building its next step
                // against a channel `Input` that may already be
                // consumed by *this* still-unconfirmed tx -- a real,
                // previously-hit preprod failure (a `Close` silently
                // never landing after an immediately-preceding `Add`).
                wait_for_landing(wallet, hash, crate::sleep::sleep)
                    .await
                    .map_err(|e| Error::Wallet(e.to_string()))?;
                if ctx.basket_opens().is_empty() {
                    break hash;
                }
            }
            Err(submit_err) => {
                *ctx = snapshot;
                let message = submit_err.to_string();

                if let Some(stale) =
                    missing_input(&message).filter(|input| ctx.basket_wills().contains_key(input))
                {
                    ctx.basket_drop_intent(&stale);
                } else if too_early_retries < 10
                    && let Some(wait) = too_early_wait(&message)
                {
                    too_early_retries += 1;
                    crate::sleep::sleep(wait).await;
                } else if max_opens > 1 && !ctx.basket_opens().is_empty() {
                    max_opens = back_off(max_opens, &message);
                } else {
                    return Err(Error::Wallet(message));
                }
            }
        }
    };
    Ok(hash)
}

/// Picks the next attempted Opens count after `attempt` was rejected by
/// `err`. A `MaxTxSizeUTxO` rejection names its own exact `supplied`/
/// `expected` byte counts; per-channel cost is roughly uniform, so
/// scaling `attempt` down by `expected/supplied` (5% safety margin for
/// the non-uniform part: tag/IOU/redeemer sizes vary a little channel
/// to channel) lands much closer to the real ceiling in one step than
/// blind halving. Anything else falls back to halving.
fn back_off(attempt: usize, message: &str) -> usize {
    if let Some((supplied, expected)) = parse_max_tx_size_mismatch(message)
        && expected < supplied
    {
        let scaled = (attempt as u128 * expected as u128 * 95 / (supplied as u128 * 100)) as usize;
        return scaled.clamp(1, attempt.saturating_sub(1).max(1));
    }
    (attempt / 2).max(1)
}

/// The first run of ASCII digits found right after `needle` in
/// `message`, if any. Shared by every "parse a number out of a chain
/// rejection's own message" site below.
fn parse_u64_after(message: &str, needle: &str) -> Option<u64> {
    let start = message.find(needle)? + needle.len();
    message[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// Pulls `supplied`/`expected` out of a `MaxTxSizeUTxO
/// Mismatch {supplied: N, expected: M}` rejection's own message.
fn parse_max_tx_size_mismatch(message: &str) -> Option<(u64, u64)> {
    Some((
        parse_u64_after(message, "supplied: ")?,
        parse_u64_after(message, "expected: ")?,
    ))
}

/// The exact `Input` amaru names as missing when a tx references an
/// already-spent (or never-existed) input or reference input --
/// parsed from "...transaction input or reference input: {64 hex
/// chars}#{index}". `None` for any other rejection shape.
fn missing_input(message: &str) -> Option<cardano_sdk::Input> {
    let needle = "transaction input or reference input: ";
    let start = message.find(needle)? + needle.len();
    let rest = message.get(start..)?;
    let hash_hex = rest.get(..64)?;
    let hash: [u8; 32] = hex::decode(hash_hex).ok()?.try_into().ok()?;
    let index = parse_u64_after(rest, "#")?;
    Some(cardano_sdk::Input::new(Hash::<32>::from(hash), index))
}

/// How long to wait before `current slot` could possibly reach the
/// tx's own lower bound -- parsed from amaru's own rejection message
/// ("current slot {current} not within transaction validity interval
/// [{needed}, +∞)"), +5s buffer. `None` for any other rejection shape.
fn too_early_wait(message: &str) -> Option<Duration> {
    let current = parse_u64_after(message, "current slot ")?;
    let needed = parse_u64_after(message, "validity interval [")?;
    Some(Duration::from_secs(needed.saturating_sub(current) + 5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_off_scales_by_reported_size_mismatch() {
        let message = "MaxTxSizeUTxO Mismatch {supplied: 20000, expected: 16384}";
        assert_eq!(back_off(40, message), 31);
    }

    #[test]
    fn back_off_halves_on_an_unrecognized_rejection() {
        let message = "some other ledger rejection";
        assert_eq!(back_off(40, message), 20);
    }

    #[test]
    fn missing_input_parses_hash_and_index() {
        let hash_hex = "11".repeat(32);
        let message = format!(
            "bad inputs UTxO: transaction input or reference input: {hash_hex}#3 is missing"
        );
        let input = missing_input(&message).expect("should parse");
        assert_eq!(input.output_index(), 3);
    }

    #[test]
    fn too_early_wait_adds_five_second_buffer() {
        let message = "current slot 100 not within transaction validity interval [140, +\u{221e})";
        assert_eq!(too_early_wait(message), Some(Duration::from_secs(45)));
    }
}
