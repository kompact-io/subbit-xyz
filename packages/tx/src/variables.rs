use minicbor::{Decode, Encode};
use subbit_core::{Duration, Stage};

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Variables {
    #[n(0)]
    amount: u64,
    #[n(1)]
    stage: Stage,
}

impl Variables {
    pub fn new(amount: u64, stage: Stage) -> Self {
        Self { amount, stage }
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn add(&self, amount: u64) -> Result<Self, Error> {
        let Stage::Opened { .. } = &self.stage else {
            return Err(Error::pair(self.stage.label(), "Add"));
        };
        if amount == 0 {
            return Err(Error::AddAmount);
        }
        Ok(Self::new(self.amount + amount, self.stage.clone()))
    }

    /// Returns the new state plus the actual gain applied.
    pub fn sub(&self, iou_amount: u64) -> Result<(Self, u64), Error> {
        let Stage::Opened { subbed } = &self.stage else {
            return Err(Error::pair(self.stage.label(), "Sub"));
        };
        let rel_owed = iou_amount.checked_sub(*subbed).ok_or(Error::IouStale)?;
        if rel_owed == 0 {
            return Err(Error::IouUsed);
        }
        if self.amount == 0 {
            return Err(Error::NoFunds);
        }
        let gain = std::cmp::min(rel_owed, self.amount);
        let variables = Self::new(
            self.amount - gain,
            Stage::Opened {
                subbed: subbed + gain,
            },
        );
        Ok((variables, gain))
    }

    pub fn close(&self, upper: &Duration, close_period: &Duration) -> Result<Self, Error> {
        let Stage::Opened { subbed } = &self.stage else {
            return Err(Error::pair(self.stage.label(), "Close"));
        };
        Ok(Self::new(
            self.amount,
            Stage::Closed {
                subbed: *subbed,
                elapse_at: *upper + *close_period,
            },
        ))
    }

    pub fn elapse(&self, lower: &Duration) -> Result<(), Error> {
        let Stage::Closed { elapse_at, .. } = &self.stage else {
            return Err(Error::pair(self.stage.label(), "Elapse"));
        };
        if lower <= elapse_at {
            Err(Error::Early(*lower, *elapse_at))
        } else {
            Ok(())
        }
    }

    /// Same shape as `sub`, but transitions Closed → Settled. Zero gain is
    /// legitimate here: it still effects the transition.
    pub fn settle(&self, iou_amount: u64) -> Result<(Self, u64), Error> {
        let Stage::Closed { subbed, .. } = &self.stage else {
            return Err(Error::pair(self.stage.label(), "Settle"));
        };
        let rel_owed = iou_amount.checked_sub(*subbed).ok_or(Error::IouStale)?;
        let gain = std::cmp::min(rel_owed, self.amount);
        Ok((Self::new(self.amount - gain, Stage::Settled), gain))
    }

    pub fn end(&self) -> Result<(), Error> {
        let Stage::Settled = &self.stage else {
            return Err(Error::pair(self.stage.label(), "End"));
        };
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("wrong stage: wanted {want}, but channel is {stage}")]
    WrongStage {
        want: &'static str,
        stage: &'static str,
    },
    #[error("add amount is too small")]
    AddAmount,
    #[error("IOU is stale")]
    IouStale,
    #[error("IOU is current, but already used")]
    IouUsed,
    #[error("zero funds available")]
    NoFunds,
    #[error("too early: {0} <= {1}")]
    Early(Duration, Duration),
}

impl Error {
    pub fn pair(stage: &'static str, want: &'static str) -> Self {
        Self::WrongStage { want, stage }
    }
}
