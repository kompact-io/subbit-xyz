use cardano_sdk::{NetworkId, ProtocolParameters};

/// Container of stuff that is almost constant
/// FIXME :: Shold be able to cache and pull from file.
/// Currenctly no serde impls upstream
#[derive(Debug, Clone)]
pub struct NetworkParameters {
    pub network_id: NetworkId,
    pub protocol_parameters: ProtocolParameters,
}
