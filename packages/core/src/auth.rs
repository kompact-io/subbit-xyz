use minicbor::{Decode, Encode};

use crate::{Mac, Pop};

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Auth<S, T> {
    #[n(0)]
    Pop(#[n(0)] Pop<S>),
    #[n(1)]
    Mac(#[n(0)] Mac<T>),
}
