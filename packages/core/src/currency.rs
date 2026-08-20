use crate::{cbor, prelude::Vec};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Currency {
    Ada,
    Asset {
        #[cfg_attr(feature = "serde", serde(with = "crate::hex_bytes"))]
        hash: [u8; 28],
        #[cfg_attr(feature = "serde", serde(with = "crate::hex_bytes"))]
        name: Vec<u8>,
    },
}

impl<C> minicbor::Encode<C> for Currency {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Currency::Ada => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.array(0)?;
            }
            Currency::Asset { hash, name } => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.bytes(hash)?;
                e.bytes(name)?;
                e.end()?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Currency {
    fn decode(
        d: &mut minicbor::Decoder<'b>,
        _ctx: &mut C,
    ) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        let len = d.array()?;

        match cbor_tag.as_u64() {
            121 => {
                cbor::expect_empty(len)?;
                Ok(Currency::Ada)
            }
            122 => {
                let hash = d
                    .bytes()?
                    .try_into()
                    .map_err(|_| minicbor::decode::Error::message("expected 28-byte hash"))?;
                let name = d.bytes()?.to_vec();
                cbor::expect_end(d)?;
                Ok(Currency::Asset { hash, name })
            }
            _ => Err(minicbor::decode::Error::message("unknown Currency tag")),
        }
    }
}

#[cfg(feature = "test-utils")]
impl proptest::arbitrary::Arbitrary for Currency {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with(_: ()) -> Self::Strategy {
        use proptest::prelude::*;
        prop_oneof![
            Just(Currency::Ada),
            (
                any::<[u8; 28]>(),
                proptest::collection::vec(any::<u8>(), 1..=32)
            )
                .prop_map(|(hash, name)| Currency::Asset { hash, name }),
        ]
        .boxed()
    }
}
