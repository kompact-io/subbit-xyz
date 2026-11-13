//! One module per wire dialect: encodes the core `Stamp`'s next
//! `Request` into that dialect's own wire envelope, and decodes that
//! dialect's own response shape back into a call on the core
//! `Stamp::apply_status`. Mirrors `subbit-gate`'s own
//! `dialect::{vanilla,x402}` split: same core underneath
//! (`subbit_gate::Ctx` there, `Stamp` here), each dialect its own
//! feature-gated adapter, no dialect code reaching into the other's.

#[cfg(feature = "vanilla")]
pub mod vanilla;
#[cfg(feature = "x402")]
pub mod x402;
