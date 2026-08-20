#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{Currency, Duration, Hash28, Tag, VerifyingKey, cbor};

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Constants {
    tag: Tag,
    currency: Currency,
    iou_key: VerifyingKey,
    consumer: Hash28,
    provider: Hash28,
    close_period: Duration,
}

impl Constants {
    pub fn tag(&self) -> &Tag {
        &self.tag
    }
    pub fn currency(&self) -> &Currency {
        &self.currency
    }
    pub fn iou_key(&self) -> &VerifyingKey {
        &self.iou_key
    }
    pub fn consumer(&self) -> &Hash28 {
        &self.consumer
    }
    pub fn provider(&self) -> &Hash28 {
        &self.provider
    }
    pub fn close_period(&self) -> &Duration {
        &self.close_period
    }

    pub fn new(
        tag: Tag,
        currency: Currency,
        iou_key: VerifyingKey,
        consumer: Hash28,
        provider: Hash28,
        close_period: Duration,
    ) -> Self {
        Self {
            tag,
            currency,
            iou_key,
            consumer,
            provider,
            close_period,
        }
    }

    pub fn verify(&self, max_tag_length: usize, min_close_period: u64) -> bool {
        self.tag.len() <= max_tag_length
            && self.close_period.as_millis() >= min_close_period as u128
    }
}

impl<C> minicbor::Encode<C> for Constants {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.begin_array()?;
        e.encode_with(&self.tag, ctx)?;
        e.encode_with(&self.currency, ctx)?;
        e.encode_with(self.iou_key, ctx)?;
        e.encode_with(self.consumer, ctx)?;
        e.encode_with(self.provider, ctx)?;
        e.encode_with(self.close_period, ctx)?;
        e.end()?;
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Constants {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        d.array()?;
        let tag = d.decode_with(ctx)?;
        let currency = d.decode_with(ctx)?;
        let iou_key = d.decode_with(ctx)?;
        let consumer = d.decode_with(ctx)?;
        let provider = d.decode_with(ctx)?;
        let close_period = d.decode_with(ctx)?;
        cbor::expect_end(d)?;
        Ok(Self {
            tag,
            currency,
            iou_key,
            consumer,
            provider,
            close_period,
        })
    }
}
