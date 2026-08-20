mod prelude;

// Base types

macros::newtype_array!(VerifyingKey, 32);
macros::newtype_array!(Signature, 64);
macros::newtype_bytes!(Tag);
macros::newtype_array!(Hash28, 28);

mod duration;
pub use duration::Duration;

mod currency;
pub use currency::Currency;

// On-chain
mod constants;
pub use constants::Constants;

mod stage;
pub use stage::Stage;

mod datum;
pub use datum::Datum;

mod redeemer;
pub use redeemer::{Cont, Eol, Redeemer, Step};

mod tag_tbs;
pub use tag_tbs::TagTbs;

// Envelopes
mod iou;
pub use iou::Iou;

mod tbs;
pub use tbs::Tbs;

mod signed;
pub use signed::Signed;

/// Proof of possession
pub type Pop<B> = Signed<B, Signature>;

mod mac;
pub use mac::Mac;

pub mod auth;
pub use auth::Auth;

pub mod envelope;

// Utils
mod parse_error;
pub use parse_error::ParseError;

mod cbor;
mod chunked_bytes;
mod macros;

pub mod base64;

#[cfg(feature = "serde")]
mod hex_bytes;

// Tests
#[cfg(feature = "test-utils")]
mod roundtrip;
