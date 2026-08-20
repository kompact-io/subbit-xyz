use crate::{Signature, Tag, TagTbs};
use minicbor::{Decode, Encode};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq, Encode, Decode)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Iou {
    #[n(0)]
    amount: u64,
    #[n(1)]
    signature: Signature,
}

impl Iou {
    pub fn new(amount: u64, signature: Signature) -> Self {
        Self { amount, signature }
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    pub fn tbs(&self, tag: Tag) -> TagTbs {
        TagTbs::new(tag, self.amount)
    }
}
