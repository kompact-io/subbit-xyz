//! Reshapes a reconstructed lineage (`subbit_index_dto::Output`) into
//! what the Channels tab actually renders. The grouping is
//! `index-dto`'s own, kept as-is: one entry per `Output::channels`
//! item, i.e. one per distinct `(Constants, delegation)` partition --
//! *not* re-grouped by keytag (`iou_key`+`tag`) first. A keytag
//! recurring under a different Constants is an anomaly (reuse, or a
//! mistake), not the common shape to design the primary grouping
//! around; when it happens, it now just shows up as two separate
//! `ConstantsLines` entries, each correct on its own, rather than
//! being folded into one group that has to explain itself.
//!
//! Each Constants typically has exactly one `Line` under it -- the
//! `Vec` is there to *accommodate* the unusual case (parallel lines
//! from distinct delegation/`stake_hash` partitions), not because
//! multiple lines are the common shape. Same reasoning one level down:
//! a `LineView` surfaces at most one `finalized` item and exactly one
//! `current` item directly (very often the same Txo), rather than
//! making every caller re-derive "which item actually matters" from a
//! flat `items` list + a boundary index. `items` (full history) is
//! still here for the unusual case where there's more than those one
//! or two items worth showing.

use std::collections::BTreeMap;

use cardano_sdk::{Hash, Input};
use subbit_index_dto::{Channel, Item, Line, Output, OutputReference};
use subbit_l1_dto::Constants;
use subbit_tx::step::Can;

use crate::channels::brief::Brief;
use crate::channels::status::consumer_can;

#[derive(Debug, Clone, serde::Serialize)]
pub struct LineageView {
    pub tip_height: u64,
    pub channels: Vec<ConstantsLines>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConstantsLines {
    pub constants: Constants,
    /// Keys/provider resolved to labels where known -- the "at a
    /// glance" widget this entry renders as.
    pub brief: Brief,
    /// Typically exactly one. See module doc.
    pub lines: Vec<LineView>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LineView {
    /// The line's tip -- its most recent item. `None` only for a
    /// literally empty `Line` (see `Line`'s own doc); every line
    /// `subbit_index::lineage::reconstruct` actually produces has at
    /// least one item.
    pub current: Option<Item>,
    /// The most recent item confirmed final per `is_final`, if any --
    /// very often `current` itself (same Txo: nothing's happened to
    /// the line since it became final), and never anything later than
    /// it (finality is monotonic along a line, so there's exactly one
    /// frontier to report, not one flag per item).
    pub finalized: Option<Item>,
    /// Full history, oldest-first (including `current`/`finalized`
    /// themselves) -- only consulted to render the unusual case where
    /// a line has more than those one or two items of interest.
    pub items: Vec<Item>,
    /// `current`'s own live actions -- `channels::status::consumer_can`'s
    /// exact same decision (the consumer-relevant subset of
    /// `Channel::can()`), looked up against `live` by `current`'s own
    /// txo. `None` whenever `current` isn't live any more (spent, or no
    /// items at all) -- a frontend renders this directly rather than
    /// re-deriving "is this still actionable" from `current.stage`
    /// itself (which, `Stagish` being history-only, can't say).
    pub can: Option<Vec<Can>>,
}

fn input_of(txo_ref: OutputReference) -> Input {
    Input::new(Hash::<32>::from(txo_ref.tx_hash.to_bytes()), txo_ref.index)
}

/// `confirmation_depth_of` resolves each distinct `Constants`'
/// confirmation depth (typically a provider-`Terms` lookup, e.g.
/// `Ctx::channels_lineage`'s own caller falling back to a UI-chosen
/// default when no provider match exists); `brief_of` resolves the
/// same Constants' keys/provider into labels (`crate::channels::brief`
/// -- needs `Consumers`/`IouKeys`/`Keyring`/`Providers`). Both are
/// caller-supplied functions rather than this module reaching into
/// those stores itself, so this stays pure and independently
/// testable.
pub fn group(
    output: Output,
    confirmation_depth_of: impl Fn(&Constants) -> u32,
    brief_of: impl Fn(&Constants) -> Brief,
    live: &BTreeMap<Input, subbit_tx::Channel>,
) -> LineageView {
    let tip_height = output.tip.height;
    let channels = output
        .channels
        .into_iter()
        .map(
            |Channel {
                 constants_ext,
                 lines,
             }| {
                let constants = constants_ext.constants;
                let depth = confirmation_depth_of(&constants);
                let brief = brief_of(&constants);
                let lines: Vec<LineView> = lines
                    .into_iter()
                    .map(|line| annotate(line, tip_height, depth, live))
                    .collect();
                ConstantsLines {
                    constants,
                    brief,
                    lines,
                }
            },
        )
        .collect();

    LineageView {
        tip_height,
        channels,
    }
}

fn annotate(
    line: Line,
    tip_height: u64,
    confirmation_depth: u32,
    live: &BTreeMap<Input, subbit_tx::Channel>,
) -> LineView {
    let items = line.0;
    // Exploits the monotonic structure directly: a binary search for
    // the first non-final item, rather than an independent `is_final`
    // check per item from the caller's side.
    let split = items.partition_point(|item| {
        crate::channels::is_final(tip_height, item.created_at.height, confirmation_depth)
    });
    let finalized = (split > 0).then(|| items[split - 1].clone());
    let current = items.last().cloned();
    let can = current
        .as_ref()
        .and_then(|item| live.get(&input_of(item.txo_ref)))
        .map(consumer_can);
    LineView {
        current,
        finalized,
        items,
        can,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
    use subbit_index_dto::{OutputReference, Point, Stagish};
    use subbit_l1_dto::{Asset, Stage};

    fn constants(tag: &[u8], provider: u8) -> Constants {
        Constants::new(
            Tag::from(tag.to_vec()),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([provider; 28]),
            Duration::from_secs(900),
        )
    }

    fn item(height: u64) -> Item {
        Item {
            created_at: Point {
                slot: height,
                hash: subbit_core::Hash32::from([0u8; 32]),
                height,
            },
            txo_ref: OutputReference {
                tx_hash: subbit_core::Hash32::from([0u8; 32]),
                index: 0,
            },
            stage: Stagish::Opened {
                amount: 1,
                subbed: 0,
            },
        }
    }

    fn output(channels: Vec<Channel>, tip_height: u64) -> Output {
        Output {
            tip: Point {
                slot: tip_height,
                hash: subbit_core::Hash32::from([0u8; 32]),
                height: tip_height,
            },
            channels,
        }
    }

    fn empty_brief(_: &Constants) -> Brief {
        Brief {
            consumer_label: None,
            iou_key_label: None,
            mine: false,
            can_sign: false,
            provider_matches: Vec::new(),
        }
    }

    fn no_live() -> BTreeMap<Input, subbit_tx::Channel> {
        BTreeMap::new()
    }

    #[test]
    fn tips_live_channel_attaches_can() {
        let line = Line(vec![item(50)]);
        let mut live = BTreeMap::new();
        live.insert(
            input_of(item(50).txo_ref),
            subbit_tx::Channel::new(
                constants(b"a", 1),
                subbit_tx::Variables::new(500, Stage::Opened { subbed: 0 }),
            ),
        );
        let view = annotate(line, 100, 10, &live);
        assert_eq!(view.can, Some(vec![Can::Add, Can::Close]));
    }

    #[test]
    fn tip_not_live_has_no_can() {
        let line = Line(vec![item(50)]);
        let view = annotate(line, 100, 10, &no_live());
        assert_eq!(view.can, None);
    }

    #[test]
    fn same_keytag_under_distinct_constants_stays_two_separate_entries() {
        let c1 = constants(b"a", 9);
        let c2 = constants(b"a", 8); // same iou_key+tag, different provider -- reuse
        assert_eq!(
            (c1.iou_key(), c1.tag()),
            (c2.iou_key(), c2.tag()),
            "sanity: these really do share a keytag"
        );

        let out = output(
            vec![
                Channel {
                    constants_ext: subbit_index_dto::ConstantsExt {
                        constants: c1,
                        delegation: None,
                    },
                    lines: vec![],
                },
                Channel {
                    constants_ext: subbit_index_dto::ConstantsExt {
                        constants: c2,
                        delegation: None,
                    },
                    lines: vec![],
                },
            ],
            100,
        );

        let view = group(out, |_| 10, empty_brief, &no_live());
        assert_eq!(
            view.channels.len(),
            2,
            "no re-grouping by keytag -- index-dto's own per-Constants organization is kept as-is"
        );
    }

    #[test]
    fn finalized_is_the_frontier_item_not_every_final_one() {
        // tip=100, depth=10 -> final when tip - height >= 10, i.e. height <= 90.
        let line = Line(vec![item(50), item(80), item(95)]);
        let view = annotate(line, 100, 10, &no_live());
        assert_eq!(
            view.finalized,
            Some(item(80)),
            "the most recent final item, not item(50) too"
        );
        assert_eq!(
            view.current,
            Some(item(95)),
            "current is always the tip, final or not"
        );
    }

    #[test]
    fn no_final_items_is_none() {
        let line = Line(vec![item(95), item(99)]);
        let view = annotate(line, 100, 10, &no_live());
        assert_eq!(view.finalized, None);
        assert_eq!(view.current, Some(item(99)));
    }

    #[test]
    fn every_item_final_means_current_and_finalized_coincide() {
        let line = Line(vec![item(10), item(20)]);
        let view = annotate(line, 100, 10, &no_live());
        assert_eq!(view.finalized, Some(item(20)));
        assert_eq!(view.current, Some(item(20)));
    }
}
