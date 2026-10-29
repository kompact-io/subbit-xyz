use cardano_sdk::{Network, NetworkId, ProtocolParameters};

/// Container of stuff that is almost constant
/// FIXME :: Shold be able to cache and pull from file.
/// Currenctly no serde impls upstream
#[derive(Debug, Clone)]
pub struct NetworkParameters {
    pub network_id: NetworkId,
    pub protocol_parameters: ProtocolParameters,
}

impl NetworkParameters {
    /// Protocol parameters + network id straight from `network`'s
    /// hardcoded snapshot (`cardano_sdk::Network`'s own `From` impls) --
    /// same offline approach `subbit_claim::config::Config::
    /// network_parameters` already uses natively, just parsing `network`
    /// from a plain string first (`&str`, not a native `Config`'s own
    /// typed field) so wasm and any other frontend can call this with
    /// nothing more than a network name. See plan-subbit-console.md
    /// section A's "Decision: hard code network parameters per network."
    pub fn for_network(network: &str) -> anyhow::Result<Self> {
        let network = Network::try_from(network)?;
        Ok(Self {
            network_id: NetworkId::from(network),
            protocol_parameters: ProtocolParameters::from(network),
        })
    }
}
