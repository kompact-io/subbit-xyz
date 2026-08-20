use crate::{Iou, Signature, cbor, prelude::Vec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// Eol ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Eol {
    End,
    Elapse,
}

impl<C> minicbor::Encode<C> for Eol {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Eol::End => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.array(0)?;
            }
            Eol::Elapse => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.array(0)?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Eol {
    fn decode(
        d: &mut minicbor::Decoder<'b>,
        _ctx: &mut C,
    ) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        let result = match cbor_tag.as_u64() {
            121 => Eol::End,
            122 => Eol::Elapse,
            _ => {
                return Err(minicbor::decode::Error::message("Unknown variant"));
            }
        };
        cbor::expect_empty(d.array()?)?;
        Ok(result)
    }
}

// Cont ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Cont {
    Add,
    Sub { n: u64, sig: Signature },
    Close,
    Settle { n: u64, sig: Signature },
}

impl<C> minicbor::Encode<C> for Cont {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Cont::Add => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.array(0)?;
            }
            Cont::Sub { n, sig } => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.encode_with(n, ctx)?;
                e.encode_with(sig, ctx)?;
                e.end()?;
            }
            Cont::Close => {
                e.tag(minicbor::data::Tag::new(123))?;
                e.array(0)?;
            }
            Cont::Settle { n, sig } => {
                e.tag(minicbor::data::Tag::new(124))?;
                e.begin_array()?;
                e.encode_with(n, ctx)?;
                e.encode_with(sig, ctx)?;
                e.end()?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Cont
where
    Iou: minicbor::Decode<'b, C>,
{
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        match cbor_tag.as_u64() {
            121 => {
                cbor::expect_empty(d.array()?)?;
                Ok(Cont::Add)
            }
            122 => {
                d.array()?;
                let n: u64 = d.decode_with(ctx)?;
                let sig: Signature = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Ok(Cont::Sub { n, sig })
            }
            123 => {
                cbor::expect_empty(d.array()?)?;
                Ok(Cont::Close)
            }
            124 => {
                d.array()?;
                let n: u64 = d.decode_with(ctx)?;
                let sig: Signature = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Ok(Cont::Settle { n, sig })
            }
            _ => Err(minicbor::decode::Error::message(
                "unknown Cont CBOR tag; expected 121–124",
            )),
        }
    }
}

// Step ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Step {
    Cont(Cont),
    Eol(Eol),
}

impl Step {
    pub fn is_adaptor(&self) -> bool {
        matches!(
            self,
            Step::Cont(Cont::Sub { .. }) | Step::Cont(Cont::Settle { .. })
        )
    }

    pub fn is_consumer(&self) -> bool {
        !self.is_adaptor()
    }
}

impl<C> minicbor::Encode<C> for Step
where
    Cont: minicbor::Encode<C>,
    Eol: minicbor::Encode<C>,
{
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Step::Cont(cont) => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.begin_array()?;
                e.encode_with(cont, ctx)?;
                e.end()?;
            }
            Step::Eol(eol) => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.encode_with(eol, ctx)?;
                e.end()?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Step
where
    Cont: minicbor::Decode<'b, C>,
    Eol: minicbor::Decode<'b, C>,
{
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        match cbor_tag.as_u64() {
            121 => {
                d.array()?;
                let cont: Cont = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Ok(Step::Cont(cont))
            }
            122 => {
                d.array()?;
                let eol: Eol = d.decode_with(ctx)?;
                cbor::expect_end(d)?;
                Ok(Step::Eol(eol))
            }
            _ => Err(minicbor::decode::Error::message(
                "unknown variant for Step: expected 121 or 122",
            )),
        }
    }
}

// Redeemer ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "test-utils", derive(proptest_derive::Arbitrary))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Redeemer {
    Defer,
    Main(Vec<Step>),
    Mutual,
}

impl<C> minicbor::Encode<C> for Redeemer
where
    Step: minicbor::Encode<C>,
{
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Redeemer::Defer => {
                e.tag(minicbor::data::Tag::new(121))?;
                e.array(0)?;
            }
            Redeemer::Main(steps) => {
                e.tag(minicbor::data::Tag::new(122))?;
                e.begin_array()?;
                e.begin_array()?; // inner indefinite-length array
                for step in steps {
                    e.encode_with(step, ctx)?;
                }
                e.end()?; // close inner
                e.end()?; // close outer
            }
            Redeemer::Mutual => {
                e.tag(minicbor::data::Tag::new(123))?;
                e.array(0)?;
            }
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for Redeemer
where
    Step: minicbor::Decode<'b, C>,
{
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let cbor_tag = d.tag()?;
        match cbor_tag.as_u64() {
            121 => {
                cbor::expect_empty(d.array()?)?;
                Ok(Redeemer::Defer)
            }
            122 => {
                d.array()?;
                d.array()?; // inner is (non-empty!) indefinite-length array
                let mut steps = Vec::new();
                while d.datatype()? != minicbor::data::Type::Break {
                    steps.push(d.decode_with(ctx)?);
                }
                d.skip()?; // consume inner break
                cbor::expect_end(d)?; // consume outer break
                Ok(Redeemer::Main(steps))
            }
            123 => {
                cbor::expect_empty(d.array()?)?;
                Ok(Redeemer::Mutual)
            }
            _ => Err(minicbor::decode::Error::message(
                "unknown Redeemer CBOR tag; expected 121, 122, or 123",
            )),
        }
    }
}
