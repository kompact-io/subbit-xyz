//! UI-layer adapters over `Ctx` -- `cli` (this crate's own native
//! test-driving bin, `main.rs` is a one-line shim calling `cli::run`)
//! and `wasm` (the browser bundle). Each picks its own persistence
//! location (`cli`: a file; `wasm`: `localStorage`) and serialization
//! format; `Ctx` itself knows neither -- see `ctx.rs`'s own doc.
//!
//! `cli` is feature-gated (an opt-in native test-driving bin, same
//! reasoning as `subbit-stamp`'s own `cli` feature -- a native build
//! with the feature off is still a valid, library-only build). `wasm`
//! is gated by `target_arch` instead, not a Cargo feature: it's
//! `wasm_bindgen` glue that can only ever compile for wasm32 in the
//! first place, so the target itself is the only switch that makes
//! sense -- there's no "native build with wasm turned on" to opt into.

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
