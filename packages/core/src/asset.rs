use crate::{Hash28, cbor, prelude::Vec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Asset {
    Lovelace,
    Native {
        hash: Hash28,
        #[cfg_attr(feature = "serde", serde(with = "crate::hex_bytes"))]
        name: Vec<u8>,
    },
}

impl<C> minicbor::Encode<C> for Asset {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Asset::Lovelace => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.array(0)?;
            }
            Asset::Native { hash, name } => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.encode_with(hash, ctx)?;
                e.bytes(name)?;
                e.end()?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Asset {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        let len = d.array()?;

        match cbor_tag.as_u64() {
            121 => {
                cbor::expect_empty(len)?;
                Ok(Asset::Lovelace)
            }
            122 => {
                let hash: Hash28 = d.decode_with(ctx)?;
                let name = d.bytes()?.to_vec();
                cbor::expect_end(d)?;
                Ok(Asset::Native { hash, name })
            }
            _ => Err(minicbor::decode::Error::message("unknown Asset tag")),
        }
    }
}

#[cfg(feature = "test-utils")]
impl proptest::arbitrary::Arbitrary for Asset {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with(_: ()) -> Self::Strategy {
        use proptest::prelude::*;
        prop_oneof![
            Just(Asset::Lovelace),
            (
                any::<[u8; 28]>(),
                proptest::collection::vec(any::<u8>(), 1..=32)
            )
                .prop_map(|(hash, name)| Asset::Native {
                    hash: hash.into(),
                    name
                }),
        ]
        .boxed()
    }
}
