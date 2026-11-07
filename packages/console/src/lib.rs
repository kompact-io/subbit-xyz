//! Core logic engine of subbit console
//!
//! We have loose coupling to the host env. The host env supplies:
//!
//! - persistent storage
//! - fetch
//! - logging
//! - ui / rendering
//!
//! All other logic in console lib, and so the same across host envs.

mod error;
pub use error::Error;

mod settings;
pub use settings::{
    Consumers, IouKeys, Keyring, ProviderEntry, Providers, SigningKeyHex, WalletView, Whitelist,
    discover_history,
};

mod channels;
pub use channels::{
    Brief, ConstantsLines, Divergence, LineView, LineageView, Match, ProviderMatch, Status,
    is_final, reconcile, statuses,
};

mod tx;
pub use tx::{Basket, send_payment, submit, teardown_reference_script, upload_reference_script};

mod logs;
pub use logs::{Entry, Level, Logs};

mod sleep;

mod ctx;
pub use ctx::Ctx;

#[cfg(feature = "admin")]
mod admin;
#[cfg(feature = "admin")]
pub use admin::ClaimOutcome;

pub mod ui;
