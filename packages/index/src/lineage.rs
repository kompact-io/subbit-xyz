//! Lineage reconstruction: see `packages/index/README.md`'s "Lineage"
//! and "Validation" sections for the design this implements.
//!
//! Each poll recomputes every tracked channel's full history from
//! scratch (no cached state across polls -- see the README's
//! "stateless" design assumption).
//!
//! Deliberately doesn't attempt the README's multi-input cross-channel
//! safety fallback (if one input in a multi-channel tx is ill-formed,
//! deem every lineage that tx touched Ended): that needs the whole Tx
//! (`ReadTx`), and Floe prunes TxOs by spend time, not creation time --
//! a tx's own inputs can already be gone by the time we'd look them up.
//! The single-input mutual-tx case is still covered, for free, by the
//! `(constants, stake_hash)` partition below: a mutual spend's output
//! either doesn't match the partition key or doesn't exist, so no
//! continuing edge is ever found for it.

use std::collections::{BTreeMap, BTreeSet};

use floe_dto_http::query::{ChainPoint, Utxo};
use subbit_core::Hash28;
use subbit_index_dto::{
    Channel, ConstantsExt, Item, Line, Output, OutputReference, Point, Stagish,
};
use subbit_l1_dto::{Constants, Stage};

use crate::cardano;

/// A single TxO's lineage-relevant fields, plus whatever spent it (if
/// anything).
struct Parsed {
    txo_ref: OutputReference,
    created_at: Point,
    spent_by: Option<[u8; 32]>,
    constants: Constants,
    stake_hash: Option<Hash28>,
    stage: Stage,
    amount: u64,
}

fn chain_point_to_point(p: &ChainPoint) -> Point {
    let hash: [u8; 32] = p.hash.clone().try_into().unwrap_or([0; 32]);
    Point {
        slot: p.slot,
        hash: hash.into(),
        height: p.height,
    }
}

/// `None` for a `Utxo` that isn't a well-formed channel -- dropped
/// here, same as the README's "violating item... dropped from the
/// line": it never enters any partition, so it can't start or extend
/// one.
fn parse(utxo: &Utxo) -> Option<Parsed> {
    let parsed = cardano::parse_channel_utxo(utxo).ok()?;
    Some(Parsed {
        txo_ref: OutputReference {
            tx_hash: utxo.txo_ref.hash.into(),
            index: utxo.txo_ref.index as u64,
        },
        created_at: chain_point_to_point(&utxo.block_ref),
        spent_by: utxo.spent_by,
        constants: parsed.constants,
        stake_hash: parsed.stake_hash.map(Hash28::from),
        stage: parsed.stage,
        amount: parsed.amount,
    })
}

fn to_stagish(stage: &Stage, amount: u64) -> Stagish {
    match stage {
        Stage::Opened { subbed } => Stagish::Opened {
            amount,
            subbed: *subbed,
        },
        Stage::Closed { .. } => Stagish::Closed,
        Stage::Settled => Stagish::Settled,
    }
}

/// Builds every `Line` found among `items`, which must already share
/// one `(constants, stake_hash)` partition key.
///
/// Links a TxO to its continuing output via `spent_by`: child
/// candidates are the other items created by the tx that spent it,
/// tie-broken by lowest output index (the "ordering" the README's
/// Process section calls for). A chain's root is any item that isn't
/// itself such a child.
///
/// A chain that ends unspent, or ends on a `Closed`/`Settled` item, is
/// a legitimate end -- kept as-is. A chain that ends spent with no
/// continuing child *and* the tip was still `Opened` is an anomaly
/// (most likely a mutual spend that happened not to collide with
/// another tracked channel): collapsed to a single `Terminated` entry,
/// discarding the preceding history.
fn build_lines(items: Vec<Parsed>) -> Vec<Line> {
    let by_ref: BTreeMap<OutputReference, Parsed> =
        items.into_iter().map(|p| (p.txo_ref, p)).collect();

    let mut outputs_of_tx: BTreeMap<[u8; 32], Vec<OutputReference>> = BTreeMap::new();
    for r in by_ref.keys() {
        outputs_of_tx
            .entry(r.tx_hash.to_bytes())
            .or_default()
            .push(*r);
    }

    let mut child: BTreeMap<OutputReference, OutputReference> = BTreeMap::new();
    let mut is_child: BTreeSet<OutputReference> = BTreeSet::new();
    for (r, item) in &by_ref {
        let Some(spent_by) = item.spent_by else {
            continue;
        };
        let Some(candidates) = outputs_of_tx.get(&spent_by) else {
            continue;
        };
        if let Some(&best) = candidates.iter().min_by_key(|c| c.index) {
            child.insert(*r, best);
            is_child.insert(best);
        }
    }

    let roots: Vec<OutputReference> = by_ref
        .keys()
        .filter(|r| !is_child.contains(r))
        .copied()
        .collect();

    roots
        .into_iter()
        .map(|root| {
            let mut chain = vec![root];
            while let Some(&next) = child.get(chain.last().expect("just pushed")) {
                chain.push(next);
            }
            let tip_ref = *chain.last().expect("root always pushed");
            let tip = &by_ref[&tip_ref];
            let tip_is_closed = matches!(tip.stage, Stage::Closed { .. } | Stage::Settled);

            if tip.spent_by.is_some() && !tip_is_closed {
                Line(vec![Item {
                    created_at: tip.created_at,
                    txo_ref: tip_ref,
                    stage: Stagish::Terminated,
                }])
            } else {
                Line(
                    chain
                        .into_iter()
                        .map(|r| {
                            let item = &by_ref[&r];
                            Item {
                                created_at: item.created_at,
                                txo_ref: r,
                                stage: to_stagish(&item.stage, item.amount),
                            }
                        })
                        .collect(),
                )
            }
        })
        .collect()
}

/// Reconstructs every channel's lineage from an already-fetched batch
/// of `Utxo`s (live or spent -- see `cardano::search_history`'s own
/// doc) plus the ledger tip observed alongside them. Pure and
/// synchronous: no I/O, no `floe_client::Backend`, nothing wasm-unsafe
/// -- this is deliberately the reusable half of `build_output` below,
/// so a consumer that already has `Utxo`/`ChainPoint` data by some
/// other means (e.g. `subbit-console`'s wasm build, handed JSON from
/// `floe-client`'s *separately built* wasm bundle over plain JS -- see
/// this crate's `[features]` doc on why that bundle is never a Cargo
/// dependency edge from in here) gets the exact same reconstruction
/// `build_output` uses natively, without needing a live `Backend` at
/// all.
pub fn reconstruct(utxos: Vec<Utxo>, ledger_tip: ChainPoint) -> Output {
    let mut by_key: BTreeMap<(Constants, Option<Hash28>), Vec<Parsed>> = BTreeMap::new();

    let tip = chain_point_to_point(&ledger_tip);
    for parsed in utxos.iter().filter_map(parse) {
        by_key
            .entry((parsed.constants.clone(), parsed.stake_hash))
            .or_default()
            .push(parsed);
    }

    let channels = by_key
        .into_iter()
        .map(|((constants, delegation), items)| Channel {
            constants_ext: ConstantsExt {
                constants,
                delegation,
            },
            lines: build_lines(items),
        })
        .collect();

    Output { tip, channels }
}

/// Fetches and reconstructs every channel at the subbit validator,
/// from any provider -- provider/asset/close_period filtering is the
/// server's job, not this one (`subbit_gate::index_output`'s own
/// doc), since this `Output` may genuinely be provider-agnostic: more
/// than one provider's server can consume the same lineage report.
///
/// Generic over `impl floe_client::Backend`, not tied to `grpc::Client`
/// -- see `cardano::search`'s own doc on why this crate's read path is
/// reusable against any transport. (Not reusable from `subbit-console`'s
/// wasm build specifically, though -- see `reconstruct`'s own doc for
/// why, and use that instead there.)
pub async fn build_output(client: &mut impl floe_client::Backend) -> anyhow::Result<Output> {
    let pay_hash = <[u8; 28]>::from(subbit_tx::VALIDATOR.hash);
    let (utxos, ledger_tip) = cardano::search_history(client, pay_hash, None).await?;
    Ok(reconstruct(utxos, ledger_tip))
}

#[cfg(test)]
mod tests {
    use subbit_core::{Duration, Hash32, Tag, VerifyingKey};
    use subbit_l1_dto::Asset;

    use super::*;

    fn dummy_constants() -> Constants {
        Constants::new(
            Tag::from(b"t".as_slice()),
            Asset::Lovelace,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([0u8; 28]),
            Hash28::from([0u8; 28]),
            Duration::from_millis(0),
        )
    }

    fn hash32(b: u8) -> Hash32 {
        let mut h = [0u8; 32];
        h[0] = b;
        h.into()
    }

    fn txo(tx: u8, index: u64) -> OutputReference {
        OutputReference {
            tx_hash: hash32(tx),
            index,
        }
    }

    fn item(tx: u8, index: u64, spent_by: Option<u8>, stage: Stage, amount: u64) -> Parsed {
        Parsed {
            txo_ref: txo(tx, index),
            created_at: Point {
                slot: tx as u64,
                hash: hash32(0),
                height: tx as u64,
            },
            spent_by: spent_by.map(|b| hash32(b).to_bytes()),
            constants: dummy_constants(),
            stake_hash: None,
            stage,
            amount,
        }
    }

    #[test]
    fn single_open_line_stays_open() {
        let lines = build_lines(vec![item(1, 0, None, Stage::Opened { subbed: 0 }, 500)]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0.len(), 1);
        assert_eq!(
            lines[0].0[0].stage,
            Stagish::Opened {
                amount: 500,
                subbed: 0
            }
        );
    }

    #[test]
    fn continuation_via_spent_by_extends_the_line() {
        let items = vec![
            item(1, 0, Some(2), Stage::Opened { subbed: 0 }, 100),
            item(2, 0, None, Stage::Opened { subbed: 50 }, 150),
        ];
        let lines = build_lines(items);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0.len(), 2);
        assert_eq!(lines[0].0[0].txo_ref, txo(1, 0));
        assert_eq!(lines[0].0[1].txo_ref, txo(2, 0));
        assert_eq!(
            lines[0].0[1].stage,
            Stagish::Opened {
                amount: 150,
                subbed: 50
            }
        );
    }

    #[test]
    fn spend_with_no_continuation_from_opened_is_terminated() {
        // tx 2 spent the channel but created no recognizable continuing output.
        let items = vec![item(1, 0, Some(2), Stage::Opened { subbed: 0 }, 100)];
        let lines = build_lines(items);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0.len(), 1);
        assert_eq!(lines[0].0[0].txo_ref, txo(1, 0));
        assert_eq!(lines[0].0[0].stage, Stagish::Terminated);
    }

    #[test]
    fn spend_with_no_continuation_from_closed_is_a_legitimate_end() {
        // Settle/Elapse never produce a continuing channel output -- this
        // is the expected, legitimate end of the line, not an anomaly.
        let items = vec![item(
            1,
            0,
            Some(2),
            Stage::Closed {
                subbed: 0,
                elapse_at: Duration::from_millis(0),
            },
            100,
        )];
        let lines = build_lines(items);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0.len(), 1);
        assert_eq!(lines[0].0[0].stage, Stagish::Closed);
    }

    #[test]
    fn spend_with_no_continuation_from_settled_is_a_legitimate_end() {
        // Settled never produces a continuing channel output either --
        // also the expected, legitimate end of the line.
        let items = vec![item(1, 0, Some(2), Stage::Settled, 100)];
        let lines = build_lines(items);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0.len(), 1);
        assert_eq!(lines[0].0[0].stage, Stagish::Settled);
    }

    #[test]
    fn mimic_tie_break_picks_the_lowest_output_index() {
        let items = vec![
            item(1, 0, Some(2), Stage::Opened { subbed: 0 }, 100),
            // Two outputs of tx 2 both match this partition -- the mimic
            // case. Lowest index wins as the genuine continuation; the
            // other starts its own, independent line.
            item(2, 1, None, Stage::Opened { subbed: 0 }, 999),
            item(2, 0, None, Stage::Opened { subbed: 10 }, 200),
        ];
        let lines = build_lines(items);
        assert_eq!(lines.len(), 2);
        let genuine = lines
            .iter()
            .find(|l| l.0.len() == 2)
            .expect("the continued line");
        assert_eq!(genuine.0[1].txo_ref, txo(2, 0));
        let mimic = lines
            .iter()
            .find(|l| l.0.len() == 1)
            .expect("the mimic's own line");
        assert_eq!(mimic.0[0].txo_ref, txo(2, 1));
    }
}
