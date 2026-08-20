use cardano_sdk::{Hash, Output, Value, cbor::ToCbor};
use minicbor::{Decode, Encode};
use subbit_core::{Constants, Currency, Datum, Hash28, Iou, Stage};

use crate::{
    MIN_ADA_BUFFER, VALIDATOR, Variables, iou,
    step::{self, Want, Will, WillCont, WillEol},
};

/// Data obtained from parsing a channel
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Channel {
    #[n(0)]
    constants: Constants,
    #[n(1)]
    variables: Variables,
}

/// A channel paired with the error produced while trying to step it.
/// `try_step` and `try_will` always hand `self` back unchanged on failure,
/// so callers can inspect the channel that rejected the transition.
type Failed<T = Channel> = Box<(T, step::Error)>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum FromOutputError {
    #[error("Expect Shelley Address")]
    ShelleyAddress,
    #[error("Expect Script Payment Credential")]
    ScriptCredential,
    #[error("Expect Konduit Payment Credential")]
    KonduitCredential,
    #[error("Expect datum")]
    Datum,
    #[error("Expect Inline datum")]
    Inline,
    #[error("Failed to parse datum: {0}")]
    ParseDatum(String),
    #[error("Own hash is wrong")]
    OwnHash,
}

impl TryFrom<&Output> for Channel {
    type Error = FromOutputError;

    fn try_from(output: &Output) -> Result<Self, Self::Error> {
        let Some(address) = output.address().as_shelley() else {
            return Err(FromOutputError::ShelleyAddress);
        };
        let Some(hash) = address.payment().as_script() else {
            return Err(FromOutputError::ScriptCredential);
        };
        if hash != VALIDATOR.hash {
            return Err(FromOutputError::KonduitCredential);
        }
        let Some(datum) = output.datum() else {
            return Err(FromOutputError::Datum);
        };
        let cardano_sdk::Datum::Inline(data) = datum else {
            return Err(FromOutputError::Inline);
        };
        let Datum {
            own_hash,
            constants,
            stage,
        } = minicbor::decode(&data.to_cbor())
            .map_err(|err| FromOutputError::ParseDatum(err.to_string()))?;
        if <[u8; 28]>::from(own_hash) != <[u8; 28]>::from(VALIDATOR.hash) {
            return Err(FromOutputError::OwnHash);
        }
        let value = output.value();
        let amount = match constants.currency() {
            Currency::Ada => value.lovelace().saturating_sub(MIN_ADA_BUFFER),
            Currency::Asset { hash, name } => value
                .assets()
                .get(&Hash::<28>::from(*hash))
                .and_then(|t| t.get(name))
                .copied()
                .unwrap_or(0),
        };
        let variables = Variables::new(amount, stage);
        Ok(Self {
            constants,
            variables,
        })
    }
}

impl Channel {
    pub fn new(constants: Constants, variables: Variables) -> Self {
        Self {
            constants,
            variables,
        }
    }

    pub fn constants(&self) -> &Constants {
        &self.constants
    }

    pub fn variables(&self) -> &Variables {
        &self.variables
    }

    /// The buffered amount, expressed as a `Value` in this channel's currency.
    ///
    /// Uses `saturating_add` to mirror the `saturating_sub` used when this
    /// amount was originally derived in `TryFrom<&Output>`, avoiding a
    /// debug-mode panic / release-mode wraparound in the (unlikely) case
    /// `amount` is near `u64::MAX`.
    pub fn buffered_value(&self) -> Value<u64> {
        let amount = self.variables().amount();
        if let Currency::Asset { hash, name } = self.constants().currency() {
            Value::new(MIN_ADA_BUFFER).with_assets([((*hash).into(), [(name.clone(), amount)])])
        } else {
            Value::new(amount.saturating_add(MIN_ADA_BUFFER))
        }
    }

    pub fn datum(&self) -> Datum {
        Datum {
            own_hash: Hash28::from(<[u8; 28]>::from(VALIDATOR.hash)),
            constants: self.constants.clone(),
            stage: self.variables.stage().clone(),
        }
    }

    pub fn can(&self) -> Vec<crate::step::Can> {
        use crate::step::Can;
        match self.variables.stage() {
            Stage::Opened { .. } => {
                let available = self.variables.amount();
                vec![Can::Add, Can::Sub { available }, Can::Close]
            }
            Stage::Closed { elapse_at, .. } => {
                let available = self.variables.amount();
                vec![
                    Can::Settle {
                        before: *elapse_at,
                        available,
                    },
                    Can::Elapse { after: *elapse_at },
                ]
            }
            Stage::Settled => vec![Can::End],
        }
    }

    /// Runs `f` against current state; on success, replaces `variables` in
    /// place and returns whatever extra value `f` produced. On failure,
    /// `self` is handed back unchanged.
    fn try_step<F, T>(mut self, f: F) -> Result<(Self, T), Failed>
    where
        F: FnOnce(&Self) -> Result<(Variables, T), step::Error>,
    {
        match f(&self) {
            Ok((variables, extra)) => {
                self.variables = variables;
                Ok((self, extra))
            }
            Err(err) => Err(self.fail(err)),
        }
    }

    fn try_verify(&self, iou: &Iou) -> Result<(), iou::Error> {
        iou::verify(self.constants.iou_key(), self.constants.tag(), iou)
    }

    /// Boxes `self` up with `err`, converting `err` into `step::Error`
    fn fail<E: Into<step::Error>>(self, err: E) -> Failed {
        Box::new((self, err.into()))
    }

    pub fn try_will(self, want: Want) -> Result<Will, Failed> {
        match want {
            Want::Add { amount } => self
                .try_step(|c| Ok((c.variables.add(amount)?, ())))
                .map(|(channel, ())| Will::cont(channel, WillCont::add(amount))),

            Want::Sub { iou } => {
                self.try_verify(&iou)
                    .map_err(|err| self.clone().fail(err))?;
                self.try_step(|c| c.variables.sub(iou.amount()).map_err(step::Error::from))
                    .map(|(channel, gain)| Will::cont(channel, WillCont::sub(iou, gain)))
            }

            Want::Close { upper } => self
                .try_step(|c| Ok((c.variables.close(&upper, c.constants.close_period())?, ())))
                .map(|(channel, ())| Will::cont(channel, WillCont::close(upper))),

            Want::Settle { iou } => {
                self.try_verify(&iou)
                    .map_err(|err| self.clone().fail(err))?;
                self.try_step(|c| c.variables.settle(iou.amount()).map_err(step::Error::from))
                    .map(|(channel, gain)| Will::cont(channel, WillCont::settle(iou, gain)))
            }

            Want::Elapse { lower } => {
                self.variables
                    .elapse(&lower)
                    .map_err(|err| self.fail(err))?;
                Ok(Will::eol(WillEol::elapse(lower)))
            }

            Want::End => {
                self.variables.end().map_err(|err| self.fail(err))?;
                Ok(Will::eol(WillEol::end()))
            }
        }
    }
}
