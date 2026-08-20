use crate::{Tag, cbor};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TagTbs {
    tag: Tag,
    amount: u64,
}

impl TagTbs {
    pub fn new(tag: Tag, amount: u64) -> Self {
        Self { tag, amount }
    }

    pub fn tag(&self) -> &Tag {
        &self.tag
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn to_vec(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("Infalliable")
    }
}

impl<C> minicbor::Encode<C> for TagTbs {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.begin_array()?;
        e.encode_with(&self.tag, ctx)?;
        e.encode_with(self.amount, ctx)?;
        e.end()?;
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for TagTbs {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        d.array()?;
        let tag: Tag = d.decode_with(ctx)?;
        let amount: u64 = d.decode_with(ctx)?;
        if d.datatype()? != minicbor::data::Type::Break {
            return Err(minicbor::decode::Error::message("expected end of array"));
        }
        cbor::expect_end(d)?;
        Ok(Self { tag, amount })
    }
}

#[cfg(feature = "test-utils")]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn tbs_roundtrip(tbs in any::<TagTbs>()) {
            let encoded = minicbor::to_vec(&tbs).unwrap();
            let decoded: TagTbs = minicbor::decode(&encoded).unwrap();
            prop_assert_eq!(decoded.tag().as_ref(), tbs.tag().as_ref());
            prop_assert_eq!(decoded.amount(), tbs.amount());
        }

        #[test]
        fn tbs_encoding_is_deterministic(tbs in any::<TagTbs>()) {
            let a = minicbor::to_vec(&tbs).unwrap();
            let b = minicbor::to_vec(&tbs).unwrap();
            prop_assert_eq!(a, b);
        }

        #[test]
        fn tbs_encodes_as_indefinite_array(tbs in any::<TagTbs>()) {
            let encoded = minicbor::to_vec(&tbs).unwrap();
            prop_assert_eq!(encoded[0], 0x9f);
            prop_assert_eq!(*encoded.last().unwrap(), 0xff);
        }
    }
}
