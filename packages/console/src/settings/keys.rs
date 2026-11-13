//! Identity/key bookkeeping: `Keyring` (full signing keys) plus
//! `Consumers`/`IouKeys` (verifying-only identities for each of
//! `Constants`'s two key-shaped roles, always populated live from
//! `Keyring` -- see `keyring`'s own module doc for why one keypair can
//! serve either or both roles). Grouped under one `keys` module since
//! all three are tightly cross-referential (each container's `insert`
//! takes the others as arguments to enforce the shared "no duplicate
//! keys and/or labels" invariant) -- see each submodule's own doc for
//! the details plan-subbit-console.md section E specifies.

pub mod consumers;
pub mod iou_keys;
pub mod keyring;

pub use consumers::Consumers;
pub use iou_keys::IouKeys;
pub use keyring::{Keyring, SigningKeyHex};
