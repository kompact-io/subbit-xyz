//! `subbit-wallet`: a self-contained `Wallet` trait plus one
//! implementor, `Embedded` (keys supplied by the caller via `Store`,
//! chain I/O via a shared `floe_client::Backend`). See
//! `plan-embedded-wallet.md` for the scope and why this crate has no
//! dependency on `subbit-console`/`subbit-tx`/any other `subbit-*`
//! crate -- `Wallet`'s "complement", a CIP-30 implementor, is
//! deliberately not built here; it belongs wherever it's wasm/JS-wired
//! up, implementing the same trait.

pub mod floe;
pub mod store;
pub mod wallet;

pub use store::{Store, reveal_hex};
pub use wallet::Wallet;
pub use wallet::embedded::{Embedded, credential_of, derive_address};
pub use wallet::witness::{self, decode_single_vkey_witness};
