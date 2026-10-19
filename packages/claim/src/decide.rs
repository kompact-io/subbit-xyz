//! The actual claiming decision, kept as a pure function (no I/O) so
//! it's unit-testable in isolation -- same separation as
//! `subbit_index::naive::rows_from_channels`.

use std::collections::BTreeMap;

use cardano_sdk::Input;
use subbit_core::Duration;
use subbit_l1_dto::Stage;
use subbit_tx::Channel;
use subbit_tx::step::Want;

use crate::client::ChannelSummary;
use crate::keytag::Keytag;

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
/// `Closed` channels are the one subtle case: a channel only gets
/// `Settle`d here once the server reports `is_active: false` (meaning
/// nothing can commit further spend against it) **or** once less than
/// `panic_settle` remains before its on-chain `elapse_at` -- whichever
/// comes first. Nothing currently flips `is_active` to `false`
/// server-side (that's a separate, deliberately-deferred fix), so
/// `panic_settle` isn't an edge case here, it's the fallback that
/// guarantees a `Closed` channel still gets settled before the
/// counterparty could instead claim it via `Elapse` -- at the cost of a
/// possibly-uncaptured last-moment spend inside that window, which is
/// the deliberate trade.
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
                let past_panic = elapse_at.saturating_sub(*now) <= *cfg.panic_settle;
                if delta >= cfg.min_single && (!summary.is_active || past_panic) {
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

    fn summary(is_active: bool, amount: u64) -> ChannelSummary {
        ChannelSummary {
            is_active,
            iou: Some(iou(amount)),
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
}
