//! The server's "Downstream" responsibility: `packages/index`'s
//! README hands lineage over and leaves the choice to us -- "Index
//! hands over lineage: Server decides which to choose."
//!
//! This includes filtering by provider/asset/close_period: an
//! `Output` may be provider-agnostic (every channel at the subbit
//! validator script, from whichever provider), so it's this module's
//! job -- not the index service's -- to keep only the channels that
//! are actually this provider's own before choosing anything.
//!
//! Given the index service's full `Output` snapshot (plus each
//! channel's own most recently verified cumulative IOU, read from the
//! db), decide each matching channel's `Backing` and collect the TxO
//! refs recorded as safe to settle.

use std::collections::BTreeMap;

use subbit_core::{Asset, Duration, Hash28};
use subbit_index_dto::{Channel as IndexChannel, Item, Line, Output, OutputReference, Stagish};
use subbit_l1_dto::Constants;

use crate::{Backing, Keytag};

/// What was decided for one channel, from its full lineage.
pub struct Decision {
    pub backing: Option<Backing>,
    /// The tip TxO of every line that ended legitimately `Closed` --
    /// i.e. ready to settle, not already settled. `Settled` lines are
    /// excluded: there's nothing left to do. `Terminated` lines are
    /// deliberately excluded here too -- that state means something
    /// unexpected happened on-chain, and needs a human to look, not
    /// automated settlement.
    pub safe_to_settle: Vec<OutputReference>,
}

/// This provider's own identity, as it must appear in a channel's
/// `Constants` for this server to consider the channel its own --
/// read off `Config`/`Terms` by the caller.
pub struct ProviderFilter {
    pub provider: Hash28,
    pub asset: Asset,
    /// The minimum close period a channel must carry to qualify --
    /// same direction as the old `subbit_index::naive::is_backing`:
    /// the channel's own close period must be at least this generous.
    pub close_period: Duration,
}

fn matches_provider(constants: &Constants, filter: &ProviderFilter) -> bool {
    constants.provider() == &filter.provider
        && constants.asset() == &filter.asset
        && constants.close_period() >= &filter.close_period
}

pub(crate) fn keytag_of(constants: &Constants) -> Keytag {
    Keytag::new(*constants.iou_key(), constants.tag().clone())
}

/// A line is "open" (a backing candidate) iff its last item is still
/// `Opened`. `Closed` contributes its tip to `safe_to_settle` instead;
/// `Settled` and `Terminated` contribute nothing anywhere -- see
/// `Decision`'s own doc. An empty line (shouldn't occur, but the
/// README calls it out explicitly) is treated the same as
/// `Terminated`: nothing.
fn tip_stagish(line: &Line) -> Option<&Stagish> {
    line.0.last().map(|item| &item.stage)
}

/// A line's one candidate: its latest item old enough, relative to
/// `tip_height`, that a rollback can no longer un-happen it
/// (`finality_depth` blocks). `None` if nothing in the line clears
/// that bar yet.
///
/// A line has exactly one candidate, period: a channel's stage only
/// ever moves forward (`Opened` -> `Closed` -> `Settled`), so once a
/// line's tip is `Opened`, every earlier item in it is `Opened` too.
/// The only question a line's own history poses is "is there a point
/// here old enough yet" -- never "which point", since the newest one
/// that clears the bar is always it.
fn candidate(line: &Line, tip_height: u64, finality_depth: u64) -> Option<&Item> {
    line.0
        .iter()
        .rev()
        .find(|item| tip_height.saturating_sub(item.created_at.height) >= finality_depth)
}

/// Provable claimable funds for one candidate TxO, given the
/// channel's most recently verified cumulative IOU (`cumulative_owed`,
/// 0 if none yet): the minimum of what's provably owed beyond what's
/// already been paid out on-chain (`cumulative_owed - subbed`) and
/// what's actually still locked in the channel (`amount`).
///
/// This is exactly the validator's own `Sub`/`Settle` gain formula
/// (`variables.rs`): `Sub` shifts funds from `amount` into `subbed`
/// one-for-one, which cancels out of `cumulative_owed - subbed`, and
/// `Add` only ever grows `amount` -- so by construction, neither step
/// can move this number against the provider.
fn claimable(amount: u64, subbed: u64, cumulative_owed: u64) -> u64 {
    amount.min(cumulative_owed.saturating_sub(subbed))
}

/// Across every open line's own candidate (see `candidate`), the one
/// with the greatest `claimable` funds becomes the `Backing` -- its
/// raw `(amount, subbed)`, not the computed `claimable` number
/// itself. `lines` pools every line of every `IndexChannel` that
/// shares one `Keytag` (`choose_backings`'s own doc explains why
/// there can be more than one) -- the common single-channel,
/// single-line case just has nothing to compare. `None` if no open
/// line has a candidate yet.
fn best_opened<'a>(
    lines: impl IntoIterator<Item = &'a Line>,
    tip_height: u64,
    finality_depth: u64,
    cumulative_owed: Option<u64>,
) -> Option<Backing> {
    let cumulative_owed = cumulative_owed.unwrap_or(0);
    lines
        .into_iter()
        .filter(|line| matches!(tip_stagish(line), Some(Stagish::Opened { .. })))
        .filter_map(|line| candidate(line, tip_height, finality_depth))
        .filter_map(|item| match item.stage {
            Stagish::Opened { amount, subbed } => Some((amount, subbed)),
            // Unreachable: the tip filter above already guarantees
            // every item in this line is `Opened`.
            _ => None,
        })
        .max_by_key(|&(amount, subbed)| claimable(amount, subbed, cumulative_owed))
        .map(|(amount, subbed)| Backing::new(amount, subbed))
}

/// `channels` is every `IndexChannel` sharing one `Keytag` (see
/// `choose_backings`). Lines are pooled across all of them for both
/// the backing choice and `safe_to_settle`: a `Closed` line is safe
/// to settle regardless of which of the colliding channels it came
/// from.
fn choose_for_keytag(
    channels: &[&IndexChannel],
    tip_height: u64,
    finality_depth: u64,
    cumulative_owed: Option<u64>,
) -> Decision {
    let lines = || channels.iter().flat_map(|c| &c.lines);
    let safe_to_settle = lines()
        .filter(|line| matches!(tip_stagish(line), Some(Stagish::Closed)))
        .filter_map(|line| line.0.last())
        .map(|item| item.txo_ref)
        .collect();
    Decision {
        backing: best_opened(lines(), tip_height, finality_depth, cumulative_owed),
        safe_to_settle,
    }
}

/// One `Decision` per distinct `Keytag` among `output`'s channels that
/// are actually this provider's own (`filter`). `Keytag` is derived
/// from `(iou_key, tag)` alone (`keytag_of`), coarser than the
/// `(Constants, stake_hash)` lineage itself partitions by -- so more
/// than one `IndexChannel` can legitimately share one `Keytag` (a
/// mimic differing in some other `Constants` field), and all of them
/// must be chosen among together, not overwrite each other.
/// `cumulative_owed` is each matching keytag's current,
/// already-verified IOU amount -- the caller's job to supply (reading
/// it off whatever's already stored before this new `Output` changes
/// anything).
pub fn choose_backings(
    output: &Output,
    filter: &ProviderFilter,
    finality_depth: u64,
    cumulative_owed: &BTreeMap<Keytag, u64>,
) -> BTreeMap<Keytag, Decision> {
    let mut by_keytag: BTreeMap<Keytag, Vec<&IndexChannel>> = BTreeMap::new();
    for channel in &output.channels {
        if matches_provider(&channel.constants_ext.constants, filter) {
            by_keytag
                .entry(keytag_of(&channel.constants_ext.constants))
                .or_default()
                .push(channel);
        }
    }
    by_keytag
        .into_iter()
        .map(|(keytag, channels)| {
            let owed = cumulative_owed.get(&keytag).copied();
            let decision = choose_for_keytag(&channels, output.tip.height, finality_depth, owed);
            (keytag, decision)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use subbit_core::{Duration, Hash28, Hash32, Tag, VerifyingKey};
    use subbit_index_dto::{ConstantsExt, Item, Point};
    use subbit_l1_dto::Asset;

    use super::*;

    fn constants() -> Constants {
        Constants::new(
            Tag::from(b"t".as_slice()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([0u8; 28]),
            Hash28::from([0u8; 28]),
            Duration::from_millis(0),
        )
    }

    fn point(height: u64) -> Point {
        Point {
            slot: height,
            hash: Hash32::from([0u8; 32]),
            height,
        }
    }

    fn txo(index: u64) -> OutputReference {
        OutputReference {
            tx_hash: Hash32::from([0u8; 32]),
            index,
        }
    }

    fn opened_item(index: u64, height: u64, amount: u64, subbed: u64) -> Item {
        Item {
            created_at: point(height),
            txo_ref: txo(index),
            stage: Stagish::Opened { amount, subbed },
        }
    }

    fn channel(lines: Vec<Line>) -> IndexChannel {
        IndexChannel {
            constants_ext: ConstantsExt {
                constants: constants(),
                delegation: None,
            },
            lines,
        }
    }

    #[test]
    fn single_line_uses_its_newest_finalized_item() {
        // tip_height 100, finality_depth 10: index 1 (height 95) is
        // too recent; index 0 (height 50) is the newest that clears
        // the bar, and is the line's only candidate.
        let line = Line(vec![opened_item(0, 50, 500, 0), opened_item(1, 95, 900, 0)]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, Some(Backing::new(500, 0)));
        assert!(decision.safe_to_settle.is_empty());
    }

    #[test]
    fn no_item_clears_depth_means_no_backing() {
        let line = Line(vec![opened_item(0, 95, 500, 0)]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, None);
    }

    #[test]
    fn across_lines_picks_the_greatest_claimable_funds() {
        // Two open lines' own (single, already-finalized) candidates.
        // Line a: amount=600, subbed=0 -- nothing owed has been paid
        // out yet, so its claimable is bounded by its own amount: 600.
        // Line b: amount=500, subbed=400 -- 400 of what's owed is
        // already paid, so its claimable is bounded by the owed
        // remainder: min(500, 700-400) = 300. a wins.
        let a = Line(vec![opened_item(0, 50, 600, 0)]);
        let b = Line(vec![opened_item(1, 50, 500, 400)]);
        let decision = choose_for_keytag(&[&channel(vec![a, b])], 100, 10, Some(700));
        assert_eq!(decision.backing, Some(Backing::new(600, 0)));
    }

    #[test]
    fn claimable_is_capped_by_what_remains_in_the_channel() {
        // owed remainder (1000-0=1000) exceeds amount (500) -> capped
        // at the physical amount, not the owed figure.
        let line = Line(vec![opened_item(0, 50, 500, 0)]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(1_000));
        assert_eq!(decision.backing, Some(Backing::new(500, 0)));
    }

    #[test]
    fn closed_line_is_recorded_safe_to_settle_not_backing() {
        let line = Line(vec![
            opened_item(0, 50, 500, 0),
            Item {
                created_at: point(60),
                txo_ref: txo(1),
                stage: Stagish::Closed,
            },
        ]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, None);
        assert_eq!(decision.safe_to_settle, vec![txo(1)]);
    }

    #[test]
    fn settled_line_is_neither_backing_nor_safe_to_settle() {
        let line = Line(vec![
            opened_item(0, 50, 500, 0),
            Item {
                created_at: point(60),
                txo_ref: txo(1),
                stage: Stagish::Settled,
            },
        ]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, None);
        assert!(decision.safe_to_settle.is_empty());
    }

    #[test]
    fn terminated_line_is_neither_backing_nor_safe_to_settle() {
        let line = Line(vec![Item {
            created_at: point(50),
            txo_ref: txo(0),
            stage: Stagish::Terminated,
        }]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, None);
        assert!(decision.safe_to_settle.is_empty());
    }

    #[test]
    fn channel_with_no_lines_is_treated_as_terminated() {
        let decision = choose_for_keytag(&[&channel(vec![])], 100, 10, Some(10_000));
        assert_eq!(decision.backing, None);
        assert!(decision.safe_to_settle.is_empty());
    }

    #[test]
    fn no_iou_yet_still_backs_from_the_sole_candidate() {
        // No cumulative_owed on file -- claimable is 0 either way, but
        // with only one line there's nothing to compare against, so
        // its candidate still becomes the backing.
        let line = Line(vec![opened_item(0, 50, 500, 0)]);
        let decision = choose_for_keytag(&[&channel(vec![line])], 100, 10, None);
        assert_eq!(decision.backing, Some(Backing::new(500, 0)));
    }

    fn our_filter() -> ProviderFilter {
        ProviderFilter {
            provider: Hash28::from([0u8; 28]),
            asset: subbit_l2_dto::Asset::Lovelace,
            close_period: Duration::from_millis(0),
        }
    }

    fn output(channels: Vec<IndexChannel>) -> Output {
        Output {
            tip: point(100),
            channels,
        }
    }

    #[test]
    fn choose_backings_keeps_a_channel_matching_our_provider() {
        let line = Line(vec![opened_item(0, 50, 500, 0)]);
        let out = output(vec![channel(vec![line])]);
        let decisions = choose_backings(&out, &our_filter(), 10, &BTreeMap::new());
        assert_eq!(decisions.len(), 1);
        let keytag = keytag_of(&constants());
        assert_eq!(
            decisions.get(&keytag).unwrap().backing,
            Some(Backing::new(500, 0))
        );
    }

    #[test]
    fn choose_backings_drops_a_channel_for_a_different_provider() {
        let other_provider = Constants::new(
            Tag::from(b"t".as_slice()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([0u8; 28]),
            Hash28::from([9u8; 28]), // not our_filter()'s provider
            Duration::from_millis(0),
        );
        let line = Line(vec![opened_item(0, 50, 500, 0)]);
        let other_channel = IndexChannel {
            constants_ext: ConstantsExt {
                constants: other_provider,
                delegation: None,
            },
            lines: vec![line],
        };
        let out = output(vec![other_channel]);
        let decisions = choose_backings(&out, &our_filter(), 10, &BTreeMap::new());
        assert!(decisions.is_empty());
    }

    #[test]
    fn choose_backings_drops_a_channel_whose_close_period_is_too_short() {
        let short_close_period = Constants::new(
            Tag::from(b"t".as_slice()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([0u8; 28]),
            Hash28::from([0u8; 28]),
            Duration::from_millis(0),
        );
        let mut filter = our_filter();
        filter.close_period = Duration::from_millis(1); // stricter than the channel's 0
        let line = Line(vec![opened_item(0, 50, 500, 0)]);
        let too_short_channel = IndexChannel {
            constants_ext: ConstantsExt {
                constants: short_close_period,
                delegation: None,
            },
            lines: vec![line],
        };
        let out = output(vec![too_short_channel]);
        let decisions = choose_backings(&out, &filter, 10, &BTreeMap::new());
        assert!(decisions.is_empty());
    }

    #[test]
    fn two_channels_sharing_a_keytag_are_pooled_not_overwritten() {
        // Same (iou_key, tag) as `constants()` -> same Keytag -- but a
        // different `consumer`, so lineage would have kept these as
        // two distinct `IndexChannel`s. The smaller-amount one must
        // not silently win just by being last into a map.
        let mimic = Constants::new(
            Tag::from(b"t".as_slice()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([7u8; 28]), // different consumer
            Hash28::from([0u8; 28]),
            Duration::from_millis(0),
        );
        let real = IndexChannel {
            constants_ext: ConstantsExt {
                constants: constants(),
                delegation: None,
            },
            lines: vec![Line(vec![opened_item(0, 50, 900, 0)])],
        };
        let mimic_channel = IndexChannel {
            constants_ext: ConstantsExt {
                constants: mimic,
                delegation: None,
            },
            lines: vec![Line(vec![opened_item(1, 50, 100, 0)])],
        };
        let keytag = keytag_of(&constants());
        let owed = BTreeMap::from([(keytag.clone(), 10_000)]);

        // Order shouldn't matter either way.
        for out in [
            output(vec![real.clone(), mimic_channel.clone()]),
            output(vec![mimic_channel, real]),
        ] {
            let decisions = choose_backings(&out, &our_filter(), 10, &owed);
            assert_eq!(decisions.len(), 1);
            assert_eq!(
                decisions.get(&keytag).unwrap().backing,
                Some(Backing::new(900, 0))
            );
        }
    }
}
