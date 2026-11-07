//! One top-level `Error` for all of `subbit-console`'s own Rust logic
//! (Keyring, Consumers, IouKeys, Providers, Basket) -- see
//! plan-subbit-console.md section H. Never unwound into a panic:
//! `ui::wasm` maps every variant into a `JsError` at the boundary.
//! Storage errors (a missing/corrupt file, a failed `localStorage`
//! call) are each `ui` adapter's own concern, not represented here --
//! `ui::cli` maps `std::io::Error` via `Io`/`serde_json::Error` via
//! `Json` already covers the file case; `ui::wasm` builds a `JsError`
//! directly for its own `localStorage` failures, since this crate's
//! own `Error` doesn't need to know `localStorage` exists.
//!
//! No `anyhow` anywhere in this crate -- every fallible lib-side call
//! site returns this typed `Error` (a foreign error gets
//! `.to_string()`'d into whichever variant fits, same as
//! `Wallet`/`Basket` above), so a caller always gets something it can
//! match on, including the `cli` feature's own binary.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("label {0:?} already in use")]
    LabelTaken(String),
    #[error("key already registered under label {0:?}")]
    KeyTaken(String),
    #[error("no entry under label {0:?}")]
    LabelNotFound(String),
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("invalid key: {0}")]
    Key(String),
    #[error("invalid network: {0}")]
    Network(String),
    #[error("{0}")]
    Basket(String),
    #[error("{0}")]
    Wallet(String),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
