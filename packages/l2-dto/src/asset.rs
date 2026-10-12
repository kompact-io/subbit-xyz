use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use subbit_core::Hash28;

use crate::Network;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Asset {
    #[n(0)]
    Lovelace,
    #[n(1)]
    Native {
        #[n(0)]
        hash: Hash28,
        #[n(1)]
        #[cbor(with = "minicbor::bytes")]
        #[cfg_attr(feature = "serde", serde(with = "subbit_core::hex_bytes"))]
        name: Vec<u8>,
    },
}

impl Asset {
    /// This asset's CAIP-19 ("Asset Type and Asset ID Specification",
    /// https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-19.md)
    /// string: `{network.to_caip2()}/{asset_namespace}:{asset_reference}`.
    /// Lovelace uses `slip44:1815`, SLIP-44's own registered Cardano
    /// coin type (CAIP-20, https://github.com/satoshilabs/slips/blob/master/slip-0044.md).
    /// A native asset has no CASA-registered namespace for Cardano;
    /// `nativeasset` here is this crate's own convention, with
    /// `{hash}{hex(name)}` matching Cardano's own policy-id+asset-name
    /// "unit" convention as the reference.
    pub fn to_caip19(&self, network: Network) -> String {
        let chain_id = network.to_caip2();
        match self {
            Asset::Lovelace => format!("{chain_id}/slip44:1815"),
            Asset::Native { hash, name } => {
                format!("{chain_id}/nativeasset:{hash}{}", hex::encode(name))
            }
        }
    }
}
