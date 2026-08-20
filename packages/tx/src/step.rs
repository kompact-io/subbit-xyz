use minicbor::{Decode, Encode};
use subbit_core::{Constants, Cont, Duration, Eol, Hash28, Iou, Step};

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
        before: Duration,
        #[n(1)]
        available: u64,
    },
    #[n(4)]
    End,
    #[n(5)]
    Elapse {
        #[n(0)]
        after: Duration,
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
        upper: Duration,
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
        lower: Duration,
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
        upper: Duration,
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
    pub fn close(upper: Duration) -> Self {
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
        lower: Duration,
    },
}

impl WillEol {
    pub fn end() -> Self {
        Self::End
    }
    pub fn elapse(lower: Duration) -> Self {
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
