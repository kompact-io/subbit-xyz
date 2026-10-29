//! Everything the Settings tab renders, grouped here because the UI
//! groups it there (this crate's own rule: module hierarchy mirrors UI
//! hierarchy, not the other way round -- see `lib.rs`'s own doc).
//! `keys` (identity bookkeeping: Keyring/Consumers/IouKeys), `floe`
//! (Floe I/O glue) and `wallet` (the embedded wallet's own key/
//! delegation state) were previously split across a top-level `keys`
//! module and a top-level `backends` module (`backends::floe` +
//! `backends::wallet`) -- `backends` added a grouping layer this
//! module doesn't need (there's nothing else under `settings` that
//! *isn't* a backend/identity/config concern), so it's gone: `floe.rs`
//! and `wallet.rs` sit directly under `settings/`, flattened. `keys`
//! keeps its own `keys/consumers.rs`/`keys/iou_keys.rs`/`keys/
//! keyring.rs` split (three tightly cross-referential submodules,
//! same reasoning as before the move). `providers` (labelled provider
//! book) and `whitelist` (whitelisted validator script hashes) moved
//! in alongside them for the same reason: both are config a user sets
//! up once and rarely touches again, not domain data that changes as
//! channels progress (that's `channels`/`tx`).

pub mod floe;
pub mod keys;
pub mod providers;
pub mod wallet;
pub mod whitelist;

pub use floe::discover_history;
pub use keys::{Consumers, IouKeys, Keyring, SigningKeyHex};
pub use providers::{ProviderEntry, Providers};
pub use wallet::View as WalletView;
pub use whitelist::Whitelist;
