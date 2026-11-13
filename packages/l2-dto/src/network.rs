use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Network {
    #[n(0)]
    Mainnet,
    #[n(1)]
    Preprod,
    #[n(2)]
    Preview,
}

impl Network {
    /// This network's CIP-34 ("Chain ID Registry",
    /// https://github.com/cardano-foundation/CIPs/tree/master/CIP-0034)
    /// string: `cip34:{NetworkId}-{NetworkMagic}`, from CIP-34's own
    /// registry.json. x402's own Cardano `exact` scheme spec accepts
    /// this as an input alias for `to_caip2`'s canonical id, not as
    /// the canonical form itself (poor devex: not human-readable).
    pub fn to_cip34(&self) -> String {
        match self {
            Network::Mainnet => "cip34:1-764824073",
            Network::Preprod => "cip34:0-1",
            Network::Preview => "cip34:0-2",
        }
        .to_string()
    }

    /// This network's CAIP-2 ("Blockchain ID Specification",
    /// https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-2.md)
    /// string. `cardano` has no CASA-registered namespace, so there's
    /// no strictly "canonical" CAIP-2 form for it; x402's own Cardano
    /// `exact` scheme spec settles on these human-readable ids as the
    /// canonical one it advertises, over `to_cip34`'s own (also valid)
    /// numeric form, on developer-experience grounds.
    pub fn to_caip2(&self) -> String {
        match self {
            Network::Mainnet => "cardano:mainnet",
            Network::Preprod => "cardano:preprod",
            Network::Preview => "cardano:preview",
        }
        .to_string()
    }
}
