use crate::{Duration, cbor};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Stage {
    Opened { subbed: u64 },
    Closed { subbed: u64, elapse_at: Duration },
    Settled,
}

impl Stage {
    pub fn is_opened(&self) -> bool {
        matches!(self, Stage::Opened { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Stage::Opened { .. } => "Opened",
            Stage::Closed { .. } => "Closed",
            Stage::Settled => "Settled",
        }
    }
}

impl<C> minicbor::Encode<C> for Stage {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Stage::Opened { subbed } => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.begin_array()?;
                e.encode_with(subbed, ctx)?;
                e.end()?;
            }
            Stage::Closed { subbed, elapse_at } => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.encode_with(subbed, ctx)?;
                e.encode_with(elapse_at, ctx)?;
                e.end()?;
            }
            Stage::Settled => {
                e.tag(minicbor::data::Tag::new(123))?;
                e.array(0)?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Stage {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        let len = d.array()?;

        let result = match cbor_tag.as_u64() {
            121 => {
                let subbed: u64 = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Stage::Opened { subbed }
            }
            122 => {
                let subbed: u64 = d.decode_with(ctx)?;
                let elapse_at: Duration = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Stage::Closed { subbed, elapse_at }
            }
            123 => {
                cbor::expect_empty(len)?;
                Stage::Settled
            }
            _ => {
                return Err(minicbor::decode::Error::message(
                    "unknown Stage CBOR tag; expected 121, 122, or 123",
                ));
            }
        };

        Ok(result)
    }
}
