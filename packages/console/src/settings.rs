//! Everything the Settings tab renders, grouped here because the UI
//! groups it there (this crate's own rule: module hierarchy mirrors UI
//! hierarchy, not the other way round -- see `lib.rs`'s own doc).
//! `keys` (identity bookkeeping: Keyring/Consumers/IouKeys, split into
//! its own `keys/consumers.rs`/`keys/iou_keys.rs`/`keys/keyring.rs`
//! since the three are tightly cross-referential), `floe` (Floe I/O
//! glue), `wallet` (the embedded wallet's own key/delegation state),
//! `providers` (labelled provider book) and `whitelist` (whitelisted
//! validator script hashes) all sit flat under `settings/`: each is
//! config a user sets up once and rarely touches again, not domain
//! data that changes as channels progress (that's `channels`/`tx`).

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
