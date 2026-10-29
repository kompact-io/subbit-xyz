//! The actual claiming decision, kept as a pure function (no I/O) so
//! it's unit-testable in isolation -- same separation as
//! `subbit_index::naive::rows_from_channels`.

use std::collections::BTreeMap;

use cardano_sdk::Input;
use subbit_core::Duration;
use subbit_index_dto::OutputReference;
use subbit_l1_dto::Stage;
use subbit_tx::Channel;
use subbit_tx::step::Want;

use crate::client::ChannelSummary;
use crate::keytag::Keytag;

fn output_ref(input: &Input) -> OutputReference {
    OutputReference {
        tx_hash: <[u8; 32]>::from(input.transaction_id()).into(),
        index: input.output_index(),
    }
}

pub struct DecisionConfig {
    /// Skip a tick's tx entirely unless the sum of its Sub/Settle
    /// candidates' claimable deltas clears this.
    pub min_total: u64,
    /// Drop a channel from this tick's Sub/Settle candidates if its own
    /// claimable delta doesn't clear this.
    pub min_single: u64,
    /// See this function's own doc comment.
    pub panic_settle: Duration,
}

/// Decides what to batch into this tick's transaction. `channels` must
/// already be filtered to this service's own provider identity;
/// `summaries` is keyed the same way (`Keytag::of`).
///
/// `Closed` channels are the one subtle case: settling early risks
/// colliding with whatever UTxO the index/server's lineage currently
/// treats as the live backing. A `Closed` input is confirmed safe to
/// settle now when either the server reports no open-line backing at
/// all for this keytag (`summary.backing: None` -- nothing else is
/// contesting it) or this exact input's own txo ref is in
/// `summary.safe_to_settle` (the server's lineage traced it to a
/// legitimately closed line). Failing that, this still falls back to
/// the older, less-informed rule: the server reports `is_active:
/// false` (nothing can commit further spend), or less than
/// `panic_settle` remains before the on-chain `elapse_at` -- the
/// guarantee that a `Closed` channel still gets settled before the
/// counterparty could instead claim it via `Elapse`, at the cost of a
/// possibly-uncaptured last-moment spend inside that window.
///
/// Never produces `Want::End`: `End`'s required signer is the
/// *consumer* (`Will::signer` only treats `Sub`/`Settle` as provider-
/// signed -- see `packages/tx/src/step.rs`'s `WillCont::is_provider`),
/// since `End` reclaims whatever deposit remains back to the consumer,
/// not the provider. A provider-only key can't sign it, so it isn't
/// this function's job to propose it.
pub fn decide(
    channels: &BTreeMap<Input, Channel>,
    summaries: &BTreeMap<Keytag, ChannelSummary>,
    cfg: &DecisionConfig,
    now: Duration,
) -> Vec<(Input, Want)> {
    let mut claims: Vec<(Input, Want, u64)> = Vec::new();

    for (input, channel) in channels {
        let keytag = Keytag::of(channel.constants());
        let Some(summary) = summaries.get(&keytag) else {
            continue;
        };
        let Some(iou) = &summary.iou else {
            continue;
        };

        match channel.variables().stage() {
            Stage::Opened { subbed } => {
                let delta = iou.amount().saturating_sub(*subbed);
                if delta >= cfg.min_single {
                    claims.push((input.clone(), Want::Sub { iou: iou.clone() }, delta));
                }
            }
            Stage::Closed { subbed, elapse_at } => {
                let delta = iou.amount().saturating_sub(*subbed);
                let confirmed_safe = summary.backing.is_none()
                    || summary.safe_to_settle.contains(&output_ref(input));
                let past_panic = elapse_at.saturating_sub(*now) <= *cfg.panic_settle;
                if delta >= cfg.min_single && (confirmed_safe || !summary.is_active || past_panic) {
                    claims.push((input.clone(), Want::Settle { iou: iou.clone() }, delta));
                }
            }
            Stage::Settled => {}
        }
    }

    let total: u64 = claims.iter().map(|(_, _, delta)| *delta).sum();
    if total >= cfg.min_total {
        claims.into_iter().map(|(i, w, _)| (i, w)).collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use cardano_sdk::Hash;
    use subbit_core::{Duration, Iou, Signature, Tag, VerifyingKey};
    use subbit_l1_dto::{Asset, Constants};
    use subbit_tx::Variables;

    use super::*;

    fn input(seed: u8) -> Input {
        Input::new(Hash::<32>::from([seed; 32]), 0)
    }

    fn iou(amount: u64) -> Iou {
        Iou::new(amount, Signature::from([0u8; 64]))
    }

    fn channel(stage: Stage) -> Channel {
        let constants = Constants::new(
            Tag::from(b"t".to_vec()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            subbit_core::Hash28::from([1u8; 28]),
            subbit_core::Hash28::from([2u8; 28]),
            Duration::from_secs(900),
        );
        Channel::new(constants, Variables::new(10_000_000, stage))
    }

    /// `backing: Some(..)` (an unrelated placeholder value, not
    /// `None`) and an empty `safe_to_settle`: this channel is neither
    /// of the two new "confirmed safe" cases, so tests built on this
    /// helper exercise only the older `is_active`/`panic_settle`
    /// fallback -- see `confirmed_safe_from_no_backing_settles_a_closed_channel_immediately`
    /// and its sibling below for the new rule itself.
    fn summary(is_active: bool, amount: u64) -> ChannelSummary {
        ChannelSummary {
            is_active,
            iou: Some(iou(amount)),
            safe_to_settle: Vec::new(),
            backing: Some(crate::client::Backing {
                amount: 0,
                subbed: 0,
            }),
        }
    }

    fn cfg(min_total: u64, min_single: u64, panic_settle_secs: u64) -> DecisionConfig {
        DecisionConfig {
            min_total,
            min_single,
            panic_settle: Duration::from_secs(panic_settle_secs),
        }
    }

    fn channels_summaries(
        channel: Channel,
        summary: ChannelSummary,
    ) -> (BTreeMap<Input, Channel>, BTreeMap<Keytag, ChannelSummary>) {
        let keytag = Keytag::of(channel.constants());
        let inp = input(9);
        (
            BTreeMap::from([(inp, channel)]),
            BTreeMap::from([(keytag, summary)]),
        )
    }

    #[test]
    fn opened_below_min_single_is_dropped() {
        let (channels, summaries) =
            channels_summaries(channel(Stage::Opened { subbed: 0 }), summary(true, 100));
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1_000, 0),
            Duration::from_secs(0),
        );
        assert!(wants.is_empty());
    }

    #[test]
    fn below_min_total_drops_everything() {
        let (channels, summaries) =
            channels_summaries(channel(Stage::Opened { subbed: 0 }), summary(true, 1_000));
        let wants = decide(
            &channels,
            &summaries,
            &cfg(1_000_000, 1, 0),
            Duration::from_secs(0),
        );
        assert!(wants.is_empty());
    }

    #[test]
    fn settled_channels_are_never_claimed() {
        let (channels, summaries) = channels_summaries(channel(Stage::Settled), summary(true, 0));
        let wants = decide(&channels, &summaries, &cfg(0, 1, 0), Duration::from_secs(0));
        assert!(wants.is_empty());
    }

    #[test]
    fn closed_active_with_time_left_waits() {
        let elapse_at = Duration::from_secs(1_000);
        let (channels, summaries) = channels_summaries(
            channel(Stage::Closed {
                subbed: 0,
                elapse_at,
            }),
            summary(true, 1_000),
        );
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1, 60),
            Duration::from_secs(0),
        );
        assert!(wants.is_empty());
    }

    #[test]
    fn closed_active_inside_panic_window_settles_anyway() {
        let elapse_at = Duration::from_secs(30);
        let (channels, summaries) = channels_summaries(
            channel(Stage::Closed {
                subbed: 0,
                elapse_at,
            }),
            summary(true, 1_000),
        );
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1, 60),
            Duration::from_secs(0),
        );
        assert_eq!(wants.len(), 1);
        assert!(matches!(wants[0].1, Want::Settle { .. }));
    }

    #[test]
    fn closed_inactive_settles_regardless_of_time_left() {
        let elapse_at = Duration::from_secs(1_000);
        let (channels, summaries) = channels_summaries(
            channel(Stage::Closed {
                subbed: 0,
                elapse_at,
            }),
            summary(false, 1_000),
        );
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1, 60),
            Duration::from_secs(0),
        );
        assert_eq!(wants.len(), 1);
        assert!(matches!(wants[0].1, Want::Settle { .. }));
    }

    #[test]
    fn confirmed_safe_from_no_backing_settles_a_closed_channel_immediately() {
        // Active, plenty of time left -- the old rule alone would
        // wait (see `closed_active_with_time_left_waits`). No backing
        // on file for this keytag at all is, by itself, enough.
        let elapse_at = Duration::from_secs(1_000);
        let (channels, mut summaries) = channels_summaries(
            channel(Stage::Closed {
                subbed: 0,
                elapse_at,
            }),
            summary(true, 1_000),
        );
        for summary in summaries.values_mut() {
            summary.backing = None;
        }
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1, 60),
            Duration::from_secs(0),
        );
        assert_eq!(wants.len(), 1);
        assert!(matches!(wants[0].1, Want::Settle { .. }));
    }

    #[test]
    fn confirmed_safe_from_safe_to_settle_settles_a_closed_channel_immediately() {
        // Same setup, but this time there *is* a backing on file
        // (elsewhere) -- it's specifically this input's own txo ref
        // showing up in `safe_to_settle` that makes it safe.
        let elapse_at = Duration::from_secs(1_000);
        let (channels, mut summaries) = channels_summaries(
            channel(Stage::Closed {
                subbed: 0,
                elapse_at,
            }),
            summary(true, 1_000),
        );
        for summary in summaries.values_mut() {
            summary.safe_to_settle = vec![output_ref(&input(9))];
        }
        let wants = decide(
            &channels,
            &summaries,
            &cfg(0, 1, 60),
            Duration::from_secs(0),
        );
        assert_eq!(wants.len(), 1);
        assert!(matches!(wants[0].1, Want::Settle { .. }));
    }
}
