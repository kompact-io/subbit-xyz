mod variables;
pub use variables::Variables;

mod bounds;
pub use bounds::Bounds;

mod channel;
pub use channel::Channel;

mod open;
pub use open::Open;

pub mod accounting;
pub mod queue;
pub mod step;

pub mod iou;

mod network_parameters;
pub use network_parameters::NetworkParameters;

pub mod ops;

mod utxos;
pub use utxos::Utxos;

pub mod validator;
pub use validator::{MIN_ADA_BUFFER, VALIDATOR};

pub mod fuel;

pub type Lovelace = u64;

pub const FEE_BUFFER: Lovelace = 3_000_000;

/// Plutus data minicbor type coersion -- shared by `queue`'s own
/// assembly and `Channel`/`Open`'s `size_estimate`.
pub(crate) fn to_plutus_data(value: impl minicbor::Encode<()>) -> cardano_sdk::PlutusData<'static> {
    let bytes = minicbor::to_vec(value).expect("encode");
    minicbor_old::decode(&bytes).expect("valid cbor round-trips into PlutusData")
}

// Utils
// TODO :: No longer usign this.
// Might want to later if upstream gets encode decode
pub mod old_cbor_bridge;
