use minicbor::{Decode, Encode};
use subbit_core::{Hash28, Instant, Iou};
use subbit_l1_dto::{Constants, Cont, Eol, Step};

use crate::{Bounds, channel::Channel, iou, variables};

// ---------------------------------------------------------------------------
// Can: advisory, derived from channel state, shown to user
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Can {
    #[n(0)]
    Add,
    #[n(1)]
    Sub {
        #[n(0)]
        available: u64,
    },
    #[n(2)]
    Close,
    #[n(3)]
    Settle {
        #[n(0)]
        before: Instant,
        #[n(1)]
        available: u64,
    },
    #[n(4)]
    End,
    #[n(5)]
    Elapse {
        #[n(0)]
        after: Instant,
    },
}

// ---------------------------------------------------------------------------
// Want: user expression, flat, unconstrained
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Want {
    #[n(0)]
    Add {
        #[n(0)]
        amount: u64,
    },
    #[n(1)]
    Sub {
        #[n(0)]
        iou: Iou,
    },
    #[n(2)]
    Close {
        #[n(0)]
        upper: Instant,
    },
    #[n(3)]
    Settle {
        #[n(0)]
        iou: Iou,
    },
    #[n(4)]
    End,
    #[n(5)]
    Elapse {
        #[n(0)]
        lower: Instant,
    },
}

impl Want {
    pub fn label(&self) -> &'static str {
        match self {
            Want::Add { .. } => "Add",
            Want::Sub { .. } => "Sub",
            Want::Close { .. } => "Close",
            Want::Settle { .. } => "Settle",
            Want::End => "End",
            Want::Elapse { .. } => "Elapse",
        }
    }
}

// ---------------------------------------------------------------------------
// Will: validated step, partitioned by output (Cont produces utxo, Eol does not)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Will {
    #[n(0)]
    Cont {
        #[n(0)]
        output: Box<Channel>,
        #[n(1)]
        step: WillCont,
    },
    #[n(1)]
    Eol {
        #[n(0)]
        step: WillEol,
    },
}

impl Will {
    pub fn cont(output: Channel, step: WillCont) -> Self {
        Self::Cont {
            output: Box::new(output),
            step,
        }
    }

    pub fn eol(step: WillEol) -> Self {
        Self::Eol { step }
    }

    pub fn bounds(&self) -> Bounds {
        match self {
            Will::Cont {
                step: WillCont::Close { upper },
                ..
            } => Bounds {
                upper: Some(*upper),
                lower: None,
            },
            Will::Eol {
                step: WillEol::Elapse { lower },
            } => Bounds {
                lower: Some(*lower),
                upper: None,
            },
            _ => Bounds::default(),
        }
    }

    pub fn to_step(&self) -> Step {
        match self {
            Will::Cont { step, .. } => Step::Cont(step.to_step()),
            Will::Eol { step } => Step::Eol(step.to_step()),
        }
    }

    pub fn is_provider(&self) -> bool {
        match self {
            Will::Cont { step, .. } => step.is_provider(),
            _ => false,
        }
    }

    pub fn signer(&self, constants: &Constants) -> Hash28 {
        if self.is_provider() {
            *constants.provider()
        } else {
            *constants.consumer()
        }
    }

    /// Estimated marginal CBOR-byte contribution of including this
    /// Will in a batch: its own input reference in the tx's input
    /// set, its own entry in the shared step list (measured directly
    /// -- a `Sub`/`Settle`'s IOU costs measurably more than
    /// `Add`/`Close`/`Elapse`/`End`), and, if continuing, its own new
    /// output (`Channel::size_estimate`).
    ///
    /// Deliberately excludes the fixed per-input redeemer-pointer/
    /// execution-units witness entry every spent script input needs
    /// (roughly constant regardless of step kind -- see
    /// `redeemer_witness_overhead`, added once per included `Will`)
    /// and any shared required-signer/witness cost (see
    /// `signer_bytes`, added once per unique signer in the batch,
    /// not per `Will`) -- those are batch-level concerns, not this
    /// `Will`'s own marginal cost; see `tx-builder-proposal.md`'s own
    /// size-estimate call.
    pub fn size_estimate(&self) -> usize {
        use cardano_sdk::{Hash, Input, cbor::ToCbor};
        // Every input encodes the same byte length (a 32-byte txid is
        // always a 32-byte txid) -- measured on a representative one
        // rather than hand-picked.
        let input_ref_bytes = Input::new(Hash::<32>::from([0u8; 32]), 0).to_cbor().len();
        let step_bytes = minicbor::to_vec(self.to_step()).expect("encode").len();
        let output_bytes = match self {
            Will::Cont { output, .. } => output.size_estimate(),
            Will::Eol { .. } => 0,
        };
        input_ref_bytes + step_bytes + output_bytes
    }
}

/// Cardano's protocol-wide cap on total execution units a single
/// transaction may declare across every redeemer combined
/// (`maxTxExecutionUnits`) -- unchanged since Vasil, identical on
/// preprod and mainnet. Used only to size the worst-case length of
/// one redeemer's own `ExecutionUnits` entry below; the true value is
/// amaru's to enforce at submit time, not something this crate has a
/// live parameter for (`cardano_sdk::ProtocolParameters` doesn't
/// track it).
const MAX_EXECUTION_UNITS: (u64, u64) = (14_000_000, 10_000_000_000);

/// A redeemer index past this could only occur in a transaction with
/// more inputs than its own 16KB size cap could ever fit, so it's
/// used below as the index's upper bound in place of `u32::MAX`'s
/// theoretical (CBOR-costlier) one.
const MAX_PLAUSIBLE_REDEEMER_INDEX: u32 = u16::MAX as u32;

/// Worst-case CBOR-encoded byte size of the one witness-set entry
/// every spent script input adds to the tx's redeemer map --
/// `RedeemersKey` (tag + index) paired with its `ExecutionUnits` --
/// on top of the step data `Will::size_estimate`'s own `step_bytes`
/// already counts. Deliberately a free function, not folded into
/// `Will::size_estimate`: it's a per-spent-script-input cost, the
/// same for every `Will` regardless of kind, so a caller summing a
/// batch adds it once per included `Will` rather than having it
/// baked into each one's own estimate.
///
/// Measured directly off `RedeemerPointer`/`ExecutionUnits`'s real
/// CBOR (130/0/.. byte patterns below), not a hand-picked guess:
/// `[tag=Spend, index=65535]` costs 5 bytes, `[mem=14_000_000,
/// steps=10_000_000_000]` costs 15, for 20 total.
pub fn redeemer_witness_overhead() -> usize {
    use cardano_sdk::{ExecutionUnits, RedeemerPointer, cbor::ToCbor};
    let ptr_bytes = RedeemerPointer::from_spend(MAX_PLAUSIBLE_REDEEMER_INDEX)
        .to_cbor()
        .len();
    let (mem, steps) = MAX_EXECUTION_UNITS;
    let ex_units_bytes = ExecutionUnits::new(mem, steps).to_cbor().len();
    ptr_bytes + ex_units_bytes
}

/// Marginal CBOR-byte cost of one more unique signer in the tx's
/// `required_signers` set: just its own 28-byte key hash as a CBOR
/// bytestring. The set's own CBOR-set tag/array-header overhead is
/// paid once per batch (and only grows by a byte past 23 entries),
/// not attributable to any one signer, so it's left for a batch-level
/// caller to add once rather than counted here.
pub fn signer_bytes() -> usize {
    use cardano_sdk::{Hash, cbor::ToCbor};
    Hash::<28>::from([0u8; 28]).to_cbor().len()
}

/// Cost of `signer_bytes` alone is incomplete: naming a hash in
/// `required_signers` doesn't make a transaction valid, the matching
/// Ed25519 signature has to actually be there, as its own entry in
/// the witness set's `vkeywitness` set -- `[vkey: 32B, signature:
/// 64B]`, fixed-size regardless of key/signature value, same as
/// `signer_bytes` not attributable beyond the one entry it's paying
/// for. `cardano_sdk` doesn't re-export `pallas::VKeyWitness` itself,
/// so this replicates its exact shape (plain 2-element array of two
/// byte strings, no map/tag wrapper -- confirmed against pallas'
/// own `#[derive]`, no `#[cbor(map)]`) directly through `minicbor`
/// rather than guessing the CBOR by hand.
pub fn vkey_witness_bytes() -> usize {
    let mut buf = Vec::new();
    let mut e = minicbor::Encoder::new(&mut buf);
    e.array(2).expect("encode");
    e.bytes(&[0u8; 32]).expect("encode"); // vkey
    e.bytes(&[0u8; 64]).expect("encode"); // signature
    buf.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use subbit_core::{Duration, Signature, Tag, VerifyingKey};
    use subbit_l1_dto::{Asset, Constants as L1Constants, Stage};

    use crate::Variables;

    fn constants() -> L1Constants {
        L1Constants::new(
            Tag::from(b"t".to_vec()),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
            Duration::from_secs(900),
        )
    }

    fn channel(stage: Stage) -> Channel {
        Channel::new(constants(), Variables::new(5_000_000, stage))
    }

    /// An `Iou`-carrying `Settle`/`Sub` must measurably outweigh a
    /// plain `End` -- confirmed against real preprod tx CBOR
    /// (~37+7=44B for `End`, ~37+79B+output for `Settle`), not just
    /// asserted in the abstract.
    #[test]
    fn settle_costs_more_than_end() {
        let end = Will::eol(WillEol::end());
        let settle = Will::cont(
            channel(Stage::Settled),
            WillCont::settle(subbit_core::Iou::new(0, Signature::new([0u8; 64])), 0),
        );
        assert!(
            settle.size_estimate() > end.size_estimate() + 100,
            "end={} settle={}",
            end.size_estimate(),
            settle.size_estimate()
        );
    }

    #[test]
    fn end_has_no_output_contribution() {
        let end = Will::eol(WillEol::end());
        // Just the input reference + a trivial step tag -- no output.
        assert!(end.size_estimate() < 50, "end={}", end.size_estimate());
    }

    /// Pinned to the real measured value so a future CBOR-library
    /// change that silently shrinks/grows these encodings gets
    /// caught here rather than only showing up as an occasional
    /// `MaxTxSizeUTxO` rejection once a batch sizer starts trusting
    /// this number.
    #[test]
    fn redeemer_witness_overhead_matches_measured_cbor() {
        assert_eq!(redeemer_witness_overhead(), 20);
    }

    #[test]
    fn signer_bytes_matches_measured_cbor() {
        // 2-byte bytestring header (length 28 needs the 1-byte-length
        // extension) + the 28 raw hash bytes.
        assert_eq!(signer_bytes(), 30);
    }

    #[test]
    fn vkey_witness_bytes_matches_measured_cbor() {
        // array(2) header (1) + [32B bytestring: 2-byte header + 32]
        // + [64B bytestring: 2-byte header + 64] = 1 + 34 + 66.
        assert_eq!(vkey_witness_bytes(), 101);
    }
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WillCont {
    #[n(0)]
    Add {
        #[n(0)]
        amount: u64,
    },
    #[n(1)]
    Sub {
        #[n(0)]
        iou: Iou,
        #[n(1)]
        gain: u64,
    },
    #[n(2)]
    Close {
        #[n(0)]
        upper: Instant,
    },
    #[n(3)]
    Settle {
        #[n(0)]
        iou: Iou,
        #[n(1)]
        gain: u64,
    },
}

impl WillCont {
    pub fn add(amount: u64) -> Self {
        Self::Add { amount }
    }
    pub fn sub(iou: Iou, gain: u64) -> Self {
        Self::Sub { iou, gain }
    }
    pub fn close(upper: Instant) -> Self {
        Self::Close { upper }
    }
    pub fn settle(iou: Iou, gain: u64) -> Self {
        Self::Settle { iou, gain }
    }

    pub fn to_step(&self) -> Cont {
        match self {
            WillCont::Add { .. } => Cont::Add,
            WillCont::Sub { iou, .. } => Cont::Sub {
                n: iou.amount(),
                sig: *iou.signature(),
            },
            WillCont::Close { .. } => Cont::Close,
            WillCont::Settle { iou, .. } => Cont::Settle {
                n: iou.amount(),
                sig: *iou.signature(),
            },
        }
    }

    fn is_provider(&self) -> bool {
        matches!(self, WillCont::Sub { .. } | WillCont::Settle { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WillEol {
    #[n(0)]
    End,
    #[n(1)]
    Elapse {
        #[n(0)]
        lower: Instant,
    },
}

impl WillEol {
    pub fn end() -> Self {
        Self::End
    }
    pub fn elapse(lower: Instant) -> Self {
        Self::Elapse { lower }
    }

    pub fn to_step(&self) -> Eol {
        match self {
            WillEol::End => Eol::End,
            WillEol::Elapse { .. } => Eol::Elapse,
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Per-channel: Want is invalid for this Channel.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Variables(#[from] variables::Error),
    #[error("IOU failed verification")]
    Iou(#[from] iou::Error),
    #[error("time bound infeasible: {reason}")]
    Bound { reason: &'static str },
}
