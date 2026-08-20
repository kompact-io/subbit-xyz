use crate::{Constants, Hash28, Stage};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Datum {
    pub own_hash: Hash28,
    pub constants: Constants,
    pub stage: Stage,
}

impl Datum {
    pub fn new(own_hash: Hash28, constants: Constants, stage: Stage) -> Self {
        Self {
            own_hash,
            constants,
            stage,
        }
    }
}

impl<C> minicbor::Encode<C> for Datum {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.begin_array()?;
        e.encode_with(self.own_hash, ctx)?;
        e.encode_with(&self.constants, ctx)?;
        e.encode_with(&self.stage, ctx)?;
        e.end()?;
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Datum {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        d.array()?;
        let own_hash: Hash28 = d.decode_with(ctx)?;
        let constants: Constants = d.decode_with(ctx)?;
        let stage: Stage = d.decode_with(ctx)?;
        d.skip()?;
        Ok(Self {
            own_hash,
            constants,
            stage,
        })
    }
}
