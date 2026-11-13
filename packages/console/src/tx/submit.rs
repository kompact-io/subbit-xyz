//! Build -> sign -> aggregate witnesses -> submit, with retry/backoff
//! on known chain rejections, draining the basket across as many
//! transactions as it takes.

use std::time::Duration;

use cardano_sdk::{Hash, Signature, VerificationKey};
use subbit_tx::Bounds;
use subbit_wallet::{Wallet, wait_for_landing};

use crate::{Ctx, Error};

/// Bound on how many "missing input" rejections a single `submit()` call will
/// absorb by dropping the stale intent and retrying, before giving up with a
/// hard error. Without this, a systemic lag between a wallet's view of a
/// just-landed batch and the node's own validation-context (observed live:
/// every candidate in a 200-channel `Close` batch hit the same
/// not-yet-recognized-input rejection) turns this into a non-terminating
/// loop -- it looks like progress (each attempt drops one more input) but
/// never actually succeeds, and neither the consumer nor the orchestrator's
/// own `MAX_LOOP_TIME` check ever gets a chance to fire, since they're
/// blocked awaiting this call the whole time.
const MAX_STALE_INTENT_RETRIES: u32 = 20;
/// Short pause before re-picking after dropping a stale intent, giving the
/// node's validation context a chance to catch up with the wallet's view --
/// mirrors `too_early_wait`'s existing "wait, don't just hammer" treatment
/// for slot-validity rejections in this same loop.
const STALE_INTENT_BACKOFF: Duration = Duration::from_secs(3);

#[tracing::instrument(skip(wallet, ctx), ret(Debug), err(Debug))]
pub async fn submit<W: Wallet>(wallet: &mut W, ctx: &mut Ctx) -> Result<Hash<32>, Error> {
    let change_address = wallet
        .change_address()
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;
    ctx.basket_set_change_address(change_address);

    if ctx.basket_opens().is_empty() && ctx.basket_wills().is_empty() {
        return Err(Error::NothingStaged);
    }

    // The drain loop shrinks `max_tx_size` to back off a size rejection; that budget is
    // this call's own and must not leak into the next one, whichever way this exits.
    let original_max_tx_size = ctx.basket_build_config().max_tx_size;
    let result = drain(wallet, ctx).await;
    let mut config = ctx.basket_build_config().clone();
    config.max_tx_size = original_max_tx_size;
    ctx.basket_set_build_config(config);
    result
}

async fn drain<W: Wallet>(wallet: &mut W, ctx: &mut Ctx) -> Result<Hash<32>, Error> {
    let original_max_tx_size = ctx.basket_build_config().max_tx_size;
    let mut too_early_retries = 0u32;
    let mut stale_intent_retries = 0u32;

    let hash = loop {
        let fuel = wallet
            .utxos(None)
            .await
            .map_err(|e| Error::Wallet(e.to_string()))?
            .ok_or_else(|| Error::Wallet("wallet reported no coverable utxos".to_string()))?;

        ctx.basket_pick(Bounds::default())?;
        let attempted_opens = ctx.basket_picked_opens().len();
        let attempted_wills = ctx.basket_picked_wills().len();
        let mut tx = match ctx.basket_build(&fuel) {
            Ok(tx) => tx,
            Err(e) => {
                ctx.basket_unpick();
                return Err(e);
            }
        };
        let id = tx.id();

        let mut wallet_witness: Option<(VerificationKey, Signature)> = None;
        let mut sign_failure = None;
        for vkh in tx.specified_signatories().collect::<Vec<_>>() {
            if let Some(local_key) = ctx
                .keyring_signing_key_for_consumer(vkh)
                .or_else(|| ctx.keyring_signing_key_for_provider(vkh))
            {
                tx.sign(&local_key);
                continue;
            }
            if wallet_witness.is_none() {
                wallet_witness = Some(match wallet.sign_tx(&tx).await {
                    Ok(witness) => witness,
                    Err(e) => {
                        ctx.basket_rollback(id);
                        return Err(Error::Wallet(e.to_string()));
                    }
                });
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
            ctx.basket_rollback(id);
            return Err(e);
        }

        let (vkey, sig) = match wallet_witness {
            Some(witness) => witness,
            None => match wallet.sign_tx(&tx).await {
                Ok(witness) => witness,
                Err(e) => {
                    ctx.basket_rollback(id);
                    return Err(Error::Wallet(e.to_string()));
                }
            },
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
                let mut config = ctx.basket_build_config().clone();
                config.max_tx_size = original_max_tx_size;
                ctx.basket_set_build_config(config);
                too_early_retries = 0;
                // Wait for confirmation before a caller re-reads chain
                // state against this still-unconfirmed tx. Deliberately
                // no finalize/rollback on a wait_for_landing failure:
                // the tx was genuinely submitted, so landing is unknown,
                // not false -- left in flight either way.
                wait_for_landing(wallet, hash, crate::sleep::sleep)
                    .await
                    .map_err(|e| Error::Wallet(e.to_string()))?;
                ctx.basket_finalize(id);
                if ctx.basket_opens().is_empty() {
                    break hash;
                }
            }
            Err(submit_err) => {
                ctx.basket_rollback(id);
                let message = submit_err.to_string();

                if let Some(stale) = missing_input(&message)
                    .filter(|input| ctx.basket_wills().contains_key(input))
                    .filter(|_| stale_intent_retries < MAX_STALE_INTENT_RETRIES)
                {
                    stale_intent_retries += 1;
                    tracing::warn!(
                        %message, %stale, stale_intent_retries,
                        "submit rejected, dropping stale intent and retrying"
                    );
                    ctx.basket_drop_intent(&stale);
                    if ctx.basket_opens().is_empty() && ctx.basket_wills().is_empty() {
                        // The dropped intent was the only thing staged -- the race that
                        // staled it already resolved everything this call meant to do.
                        return Err(Error::NothingStaged);
                    }
                    crate::sleep::sleep(STALE_INTENT_BACKOFF).await;
                } else if too_early_retries < 10
                    && let Some(wait) = too_early_wait(&message)
                {
                    too_early_retries += 1;
                    tracing::warn!(%message, ?wait, too_early_retries, "submit rejected, waiting and retrying");
                    crate::sleep::sleep(wait).await;
                } else if is_size_rejection(&message)
                    && ctx.basket_build_config().max_tx_size > 1
                    && (!ctx.basket_opens().is_empty() || !ctx.basket_wills().is_empty())
                {
                    // Shrinking the budget and re-picking (next loop iteration) drops
                    // whichever candidate is least urgent -- an Open, or (absent any) the
                    // lowest-priority Will -- per `Queue::pick`'s own shared ranking
                    // (`will_priority`/`OPEN_PRIORITY`). Gating this on Opens alone would
                    // leave a Wills-only batch with no recovery from a real chain rejection.
                    let mut config = ctx.basket_build_config().clone();
                    let next_max = back_off(config.max_tx_size, &message);
                    tracing::warn!(
                        %message,
                        previous_max = config.max_tx_size,
                        next_max,
                        "submit rejected, backing off and retrying"
                    );
                    config.max_tx_size = next_max;
                    ctx.basket_set_build_config(config);
                } else {
                    return Err(Error::Wallet(message));
                }
            }
        }
    };
    Ok(hash)
}

/// `attempt` is now the same unit (bytes) as the rejection's own
/// `supplied`/`expected` -- a `MaxTxSizeUTxO` rejection names the
/// real protocol ceiling directly (`expected`), so it's trusted
/// outright (with a small safety margin) rather than scaled
/// proportionally against the previous attempt the way a flat
/// Opens-count cap once had to be.
fn back_off(attempt: usize, message: &str) -> usize {
    if let Some((supplied, expected)) = parse_max_tx_size_mismatch(message)
        && expected < supplied
    {
        let margined = (expected as u128 * 95 / 100) as usize;
        return margined.clamp(1, attempt.saturating_sub(1).max(1));
    }
    (attempt / 2).max(1)
}

fn parse_u64_after(message: &str, needle: &str) -> Option<u64> {
    let start = message.find(needle)? + needle.len();
    message[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// Two known rejection wordings for the same `MaxTxSizeUTxO` mismatch --
/// `supplied: N, expected: N` and amaru's own `provided N bytes, maximum N bytes`
/// (confirmed live against preprod; the one `back_off`'s own smart-shrink actually
/// needs to recognize, or it falls back to blind halving on every real rejection).
fn parse_max_tx_size_mismatch(message: &str) -> Option<(u64, u64)> {
    if let (Some(supplied), Some(expected)) = (
        parse_u64_after(message, "supplied: "),
        parse_u64_after(message, "expected: "),
    ) {
        return Some((supplied, expected));
    }
    Some((
        parse_u64_after(message, "provided ")?,
        parse_u64_after(message, "maximum ")?,
    ))
}

fn missing_input(message: &str) -> Option<cardano_sdk::Input> {
    let needle = "transaction input or reference input: ";
    let start = message.find(needle)? + needle.len();
    let rest = message.get(start..)?;
    let hash_hex = rest.get(..64)?;
    let hash: [u8; 32] = hex::decode(hash_hex).ok()?.try_into().ok()?;
    let index = parse_u64_after(rest, "#")?;
    Some(cardano_sdk::Input::new(Hash::<32>::from(hash), index))
}

/// A rejection that shrinking the batch can actually cure: the tx itself was too
/// big, or its scripts overran the per-tx ExUnits budget. Anything else (validity
/// interval, bad inputs, script failure) is not fixed by picking less.
fn is_size_rejection(message: &str) -> bool {
    parse_max_tx_size_mismatch(message).is_some()
        || message.contains("MaxTxSize")
        || message.contains("ExUnits")
        || message.contains("OutOfEx")
}

/// How long to wait for a validity interval whose lower bound is still ahead of the
/// chain. `None` if the interval isn't simply early: an upper bound at or below the
/// lower bound (two wills with disjoint windows), or already behind the chain, can never
/// become valid by waiting.
fn too_early_wait(message: &str) -> Option<Duration> {
    let current = parse_u64_after(message, "current slot ")?;
    let needed = parse_u64_after(message, "validity interval [")?;
    if needed <= current {
        return None;
    }
    let upper = message
        .find("validity interval [")
        .and_then(|start| message[start..].find(", ").map(|i| start + i + 2))
        .and_then(|start| parse_u64_after(&message[start..], ""));
    if upper.is_some_and(|upper| upper <= needed) {
        return None;
    }
    Some(Duration::from_secs(needed - current + 5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_off_trusts_the_reported_ceiling_directly() {
        let message = "MaxTxSizeUTxO Mismatch {supplied: 20000, expected: 16384}";
        assert_eq!(back_off(16_384, message), 15_564);
    }

    /// amaru's own wording for the exact same rejection -- confirmed live against
    /// preprod -- which `parse_max_tx_size_mismatch`'s original pattern never matched,
    /// silently falling back to blind halving on every real rejection.
    #[test]
    fn back_off_recognizes_amarus_own_wording() {
        let message = "transaction too large: provided 16500 bytes, maximum 16384 bytes";
        assert_eq!(back_off(16_384, message), 15_564);
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

    #[test]
    fn too_early_wait_still_waits_on_a_finite_but_feasible_upper() {
        let message = "current slot 100 not within transaction validity interval [140, 200)";
        assert_eq!(too_early_wait(message), Some(Duration::from_secs(45)));
    }

    /// Observed live: a `Close`'s upper batched with an `Elapse`'s later lower.
    #[test]
    fn too_early_wait_refuses_an_inverted_interval() {
        let message = "current slot 138813459 not within transaction validity interval [138813793, 138813643)";
        assert_eq!(too_early_wait(message), None);
    }

    #[test]
    fn too_early_wait_refuses_an_expired_interval() {
        let message = "current slot 138813746 not within transaction validity interval [-\u{221e}, 138813643)";
        assert_eq!(too_early_wait(message), None);
    }

    #[test]
    fn only_size_rejections_shrink_the_batch() {
        assert!(is_size_rejection(
            "transaction too large: provided 16500 bytes, maximum 16384 bytes"
        ));
        assert!(is_size_rejection(
            "MaxTxSizeUTxO Mismatch {supplied: 20000, expected: 16384}"
        ));
        assert!(is_size_rejection("ExUnitsTooBigUTxO"));
        assert!(!is_size_rejection(
            "current slot 138813746 not within transaction validity interval [-\u{221e}, 138813643)"
        ));
        assert!(!is_size_rejection("some other ledger rejection"));
    }
}
