//! Subbit's own definition of the x402 dialect (x402 v2). `spec` is
//! x402's own generic vocabulary, reachable at its own path for
//! readers matching the spec directly; `require`/`request`/`response`
//! are Subbit's own instantiations of it, mirroring `vanilla`'s
//! `Require`/`Request`/`Response`.
//!
//! Binary fields (transactions, signatures) are hex.

pub mod spec;

mod require;
pub use require::{KnownExtra, Require, SCHEME, SubbitAccept, SubbitExtra};

mod request;
pub use request::{Payload, Request};

mod response;
pub use response::{Response, SubbitExtension, extension, settlement};
