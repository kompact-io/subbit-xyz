//! Provider-admin actions on top of the same engine every other `Ctx`
//! method uses -- gated behind the `admin` feature so a consumer-only
//! build (the default) never links or exposes this surface. Each `ui`
//! adapter wires its own admin-only widgets behind the same feature,
//! rather than this crate deciding what a UI shows.

use cardano_sdk::Input;
use subbit_core::{Iou, Signature};
use subbit_l1_dto::Stage;
use subbit_tx::step::Want;

use crate::{Ctx, Error};

/// What `Ctx::sub`/`settle`/`claim` staged, for a UI to report. `step`
/// is `Want::label()` ("Sub" or "Settle") -- which one `claim` picked,
/// or which one `sub`/`settle` were told to use directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOutcome {
    pub keytag: String,
    pub tx_oref: Input,
    pub step: &'static str,
}

impl Ctx {
    /// Every basket-known channel matching `keytag`, or just `tx_oref`
    /// directly when given (the only way to target a channel the
    /// basket doesn't locally know about yet). More than one match is
    /// possible and legitimate -- e.g. a reopened channel's lineage --
    /// not an error.
    fn resolve(&self, keytag: &str, tx_oref: Option<Input>) -> Vec<Input> {
        match tx_oref {
            Some(input) => vec![input],
            None => self
                .basket_txos()
                .iter()
                .filter(|(_, channel)| {
                    let found = channel.constants().iou_key().to_string()
                        + &channel.constants().tag().to_string();
                    found == keytag
                })
                .map(|(input, _)| input.clone())
                .collect(),
        }
    }

    /// `sub`/`settle`'s shared plumbing: stages `want` against exactly
    /// one resolved channel -- an error if `keytag`/`tx_oref` resolves
    /// to zero or more than one (ambiguous; pass `tx_oref` directly to
    /// disambiguate). Unlike `claim`, this never picks Sub vs Settle
    /// for you.
    fn stage_one(
        &mut self,
        keytag: &str,
        tx_oref: Option<Input>,
        want: Want,
    ) -> Result<ClaimOutcome, Error> {
        let mut matches = self.resolve(keytag, tx_oref);
        let tx_oref = match matches.len() {
            0 => {
                return Err(Error::Basket(format!(
                    "keytag={keytag} isn't a locally-known channel"
                )));
            }
            1 => matches.remove(0),
            n => {
                return Err(Error::Basket(format!(
                    "keytag={keytag} matches {n} locally-known channels -- pass tx_oref directly to disambiguate"
                )));
            }
        };
        let step = want.label();
        self.basket_propose(tx_oref.clone(), want)?;
        Ok(ClaimOutcome {
            keytag: keytag.to_string(),
            tx_oref,
            step,
        })
    }

    /// Stages a `Sub` want directly (a live, still-`Opened` channel's
    /// own draw-down) -- no stage check beyond what `basket_propose`
    /// itself already enforces.
    pub fn sub(
        &mut self,
        keytag: &str,
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    ) -> Result<ClaimOutcome, Error> {
        self.stage_one(
            keytag,
            tx_oref,
            Want::Sub {
                iou: Iou::new(amount, signature),
            },
        )
    }

    /// Stages a `Settle` want directly (a `Closed` channel's final
    /// payout) -- no stage check beyond what `basket_propose` itself
    /// already enforces.
    pub fn settle(
        &mut self,
        keytag: &str,
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    ) -> Result<ClaimOutcome, Error> {
        self.stage_one(
            keytag,
            tx_oref,
            Want::Settle {
                iou: Iou::new(amount, signature),
            },
        )
    }

    /// Stages whichever of `Sub`/`Settle` is actually legal, per
    /// channel, for every channel matching `keytag` (or just `tx_oref`
    /// directly) -- `Stage::Opened` gets `Sub`, `Stage::Closed` gets
    /// `Settle`, `Stage::Settled` is skipped as already done. Mirrors
    /// `subbit_claim::decide`'s own per-channel dispatch (see its
    /// `Stage::Opened`/`Closed` match), minus its economic thresholds
    /// and its "don't settle a utxo in a line still backing the
    /// channel" safety nuance -- that stays `subbit-claim`-only; this
    /// is an admin forcing it through regardless. More than one match
    /// (e.g. a reopened channel's lineage) stages a `Will` for each,
    /// in the same basket -- they land in the same tx once `basket
    /// build` runs, same as any other multi-channel batch.
    pub fn claim(
        &mut self,
        keytag: &str,
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    ) -> Vec<Result<ClaimOutcome, Error>> {
        let matches = self.resolve(keytag, tx_oref);
        if matches.is_empty() {
            return vec![Err(Error::Basket(format!(
                "keytag={keytag} isn't a locally-known channel"
            )))];
        }
        matches
            .into_iter()
            .map(|input| {
                let stage = self
                    .basket_txos()
                    .get(&input)
                    .map(|channel| channel.variables().stage().clone());
                let want = match stage {
                    Some(Stage::Opened { .. }) => Want::Sub {
                        iou: Iou::new(amount, signature),
                    },
                    Some(Stage::Closed { .. }) => Want::Settle {
                        iou: Iou::new(amount, signature),
                    },
                    Some(Stage::Settled) => {
                        return Err(Error::Basket(format!(
                            "tx_oref={input} is already Settled -- nothing to claim"
                        )));
                    }
                    None => {
                        return Err(Error::Basket(format!(
                            "tx_oref={input} vanished from the basket mid-claim"
                        )));
                    }
                };
                let step = want.label();
                self.basket_propose(input.clone(), want)?;
                Ok(ClaimOutcome {
                    keytag: keytag.to_string(),
                    tx_oref: input,
                    step,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_sdk::{Address, Hash, NetworkId, Output, address::kind};
    use std::collections::BTreeMap;
    use subbit_core::{Duration, Hash28, Tag, TagTbs, VerifyingKey};
    use subbit_l1_dto::Asset;
    use subbit_tx::{
        Channel, NetworkParameters, Variables,
        queue::{BuildConfig, Queue},
    };

    fn network_id() -> NetworkId {
        NetworkParameters::for_network("preprod")
            .unwrap()
            .network_id
    }

    /// Every test channel's `iou_key` is this same keypair's public
    /// half -- `sign` below signs with the matching secret half, so
    /// `try_verify` (real Ed25519, not stubbed) actually passes.
    fn iou_keypair() -> ([u8; 64], VerifyingKey) {
        let (sk, pk) = cryptoxide::ed25519::keypair(&[7u8; 32]);
        (sk, VerifyingKey::from(pk))
    }

    fn sign(tag_byte: u8, amount: u64) -> Signature {
        let (sk, _) = iou_keypair();
        let tbs = TagTbs::new(Tag::from(vec![tag_byte]), amount).to_vec();
        Signature::new(cryptoxide::ed25519::signature(&tbs, &sk))
    }

    fn constants(tag_byte: u8) -> subbit_l1_dto::Constants {
        subbit_l1_dto::Constants::new(
            Tag::from(vec![tag_byte]),
            Asset::Lovelace,
            iou_keypair().1,
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
            Duration::from_secs(900),
        )
    }

    /// A fake-but-real existing-channel `Output`: encodes `channel`'s
    /// own `datum()` and round-trips it into a `PlutusData`, the
    /// exact same pair `subbit_tx::queue::Queue`'s own output
    /// assembly uses for a real
    /// continuing output -- nothing admin-specific, this is just how
    /// a channel's utxo is shaped on-chain.
    fn output_for(channel: &Channel) -> Output {
        let address: Address<kind::Any> = subbit_tx::validator::address(network_id(), None).into();
        let bytes = minicbor::to_vec(channel.datum()).expect("encode datum");
        let data: cardano_sdk::PlutusData =
            minicbor_old::decode(&bytes).expect("datum round-trips into PlutusData");
        Output::new(address, channel.buffered_value()).with_datum(data)
    }

    /// Test-only: a `Queue` seeded directly with `utxos`, skipping the
    /// Floe-`Utxo`-decoding round trip real callers go through.
    fn queue_from_outputs(utxos: BTreeMap<Input, Output>) -> Queue {
        let mut queue = Queue::new(
            NetworkParameters::for_network("preprod").unwrap(),
            BuildConfig::default(),
        );
        queue
            .add_utxos(utxos)
            .expect("test utxos always parse as channels");
        queue
    }

    fn ctx_with_channels(channels: Vec<(u8, Variables)>) -> Ctx {
        let utxos: BTreeMap<Input, Output> = channels
            .into_iter()
            .enumerate()
            .map(|(i, (tag_byte, variables))| {
                let channel = Channel::new(constants(tag_byte), variables);
                let input = Input::new(Hash::<32>::from([i as u8; 32]), 0);
                (input, output_for(&channel))
            })
            .collect();
        Ctx::test_with_queue(queue_from_outputs(utxos))
    }

    fn keytag(tag_byte: u8) -> String {
        constants(tag_byte).iou_key().to_string() + &constants(tag_byte).tag().to_string()
    }

    fn opened(amount: u64) -> Variables {
        Variables::new(amount, Stage::Opened { subbed: 0 })
    }

    fn closed(amount: u64) -> Variables {
        Variables::new(
            amount,
            Stage::Closed {
                subbed: 0,
                elapse_at: subbit_core::Instant::from_secs(0),
            },
        )
    }

    #[test]
    fn claim_picks_sub_for_an_opened_channel() {
        let mut ctx = ctx_with_channels(vec![(1, opened(5_000_000))]);
        let results = ctx.claim(&keytag(1), None, 1_000, sign(1, 1_000));
        assert_eq!(results.len(), 1);
        let outcome = results[0].as_ref().expect("should stage");
        assert_eq!(outcome.step, "Sub");
    }

    #[test]
    fn claim_picks_settle_for_a_closed_channel() {
        let mut ctx = ctx_with_channels(vec![(1, closed(5_000_000))]);
        let results = ctx.claim(&keytag(1), None, 1_000, sign(1, 1_000));
        assert_eq!(results.len(), 1);
        let outcome = results[0].as_ref().expect("should stage");
        assert_eq!(outcome.step, "Settle");
    }

    #[test]
    fn claim_stages_every_match_in_the_same_basket() {
        // Two distinct channels that happen to share a keytag (the
        // same consumer/iou_key/tag, as a reopened channel's lineage
        // would) -- `claim` must stage both, each with its own
        // correctly-picked step, not just the first found.
        let ctx_input = |i: u8| Input::new(Hash::<32>::from([i; 32]), 0);
        let utxos = BTreeMap::from([
            (
                ctx_input(1),
                output_for(&Channel::new(constants(1), opened(1_000_000))),
            ),
            (
                ctx_input(2),
                output_for(&Channel::new(constants(1), closed(2_000_000))),
            ),
        ]);
        let mut ctx = Ctx::test_with_queue(queue_from_outputs(utxos));

        let results = ctx.claim(&keytag(1), None, 1_000, sign(1, 1_000));
        assert_eq!(results.len(), 2, "both matches must be staged");
        let mut steps: Vec<&str> = results
            .iter()
            .map(|r| r.as_ref().expect("should stage").step)
            .collect();
        steps.sort();
        assert_eq!(steps, ["Settle", "Sub"]);
        assert_eq!(
            ctx.basket_wills().len(),
            2,
            "both wills land in the same basket/tx"
        );
    }

    #[test]
    fn claim_reports_an_unknown_keytag() {
        let mut ctx = Ctx::default();
        let results = ctx.claim("no-such-keytag", None, 1_000, sign(1, 1_000));
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn sub_and_settle_never_pick_a_stage_for_you() {
        // `settle` against a still-Opened channel must fail exactly
        // like the direct `Want::Settle` path always has (wrong
        // stage) -- `sub`/`settle` don't inspect stage at all, unlike
        // `claim`.
        let mut ctx = ctx_with_channels(vec![(1, opened(5_000_000))]);
        let err = ctx
            .settle(&keytag(1), None, 1_000, sign(1, 1_000))
            .expect_err("Settle on an Opened channel must be rejected");
        assert!(err.to_string().contains("Settle"));
    }

    #[test]
    fn sub_and_settle_require_an_unambiguous_match() {
        let ctx_input = |i: u8| Input::new(Hash::<32>::from([i; 32]), 0);
        let utxos = BTreeMap::from([
            (
                ctx_input(1),
                output_for(&Channel::new(constants(1), opened(1_000_000))),
            ),
            (
                ctx_input(2),
                output_for(&Channel::new(constants(1), opened(2_000_000))),
            ),
        ]);
        let mut ctx = Ctx::test_with_queue(queue_from_outputs(utxos));

        let err = ctx
            .sub(&keytag(1), None, 1_000, sign(1, 1_000))
            .expect_err("ambiguous keytag must be rejected without tx_oref");
        assert!(err.to_string().contains("matches 2"));
    }
}
