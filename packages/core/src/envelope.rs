use http::StatusCode;
use minicbor::{Decode, Encode};

use crate::{Auth, Duration, Iou, Mac, Tag, VerifyingKey, tbs};

/// Basic body
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Body {
    #[n(0)]
    pub key: VerifyingKey,
    #[n(1)]
    pub tag: Tag,
    #[n(2)]
    pub ttl: Duration,
}

impl Body {
    pub fn tbs(&self) -> Vec<u8> {
        tbs::subbit(self)
    }
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub struct Request {
    #[n(0)]
    pub auth: Auth<Body, Body>,
    #[n(1)]
    pub iou: Option<Iou>,
}

pub type Response = Result<Status, Error>;

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub struct Status {
    #[n(0)]
    pub iou: Iou,
    #[n(1)]
    pub spendable: Option<u64>,
    #[n(2)]
    pub uncommitted: Option<u64>,
    #[n(3)]
    pub mac: Option<Mac<Body>>,
}

impl Status {
    pub fn status_code(&self) -> StatusCode {
        StatusCode::OK
    }
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub enum Error {
    #[n(0)]
    Other,
    #[n(1)]
    NoChannel,
    #[n(2)]
    Inactive,
    #[n(3)]
    OldAuth,
    #[n(4)]
    InvalidAuth,
    #[n(5)]
    NoIou,
    #[n(6)]
    OldIou,
    #[n(7)]
    InvalidIou,
    // Insufficient funds, or other reason
    #[n(8)]
    Unspendable,
    #[n(9)]
    RateLimited,
}

impl Error {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Error::InvalidAuth => StatusCode::UNAUTHORIZED,
            Error::OldAuth => StatusCode::UNAUTHORIZED,
            Error::NoChannel => StatusCode::NOT_FOUND,
            Error::Inactive => StatusCode::FORBIDDEN,
            Error::InvalidIou => StatusCode::BAD_REQUEST,
            Error::OldIou => StatusCode::CONFLICT,
            Error::NoIou => StatusCode::CONFLICT,
            Error::Unspendable => StatusCode::PAYMENT_REQUIRED,
            Error::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Error::Other => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
