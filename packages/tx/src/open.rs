use cardano_sdk::{Credential, Output, cbor::ToCbor};

use crate::Channel;

/// A new channel being created in this batch: the channel state itself,
/// plus the optional stake credential its output address should carry.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Open {
    channel: Channel,
    delegation: Option<Credential>,
}

impl Open {
    pub fn new(channel: Channel, delegation: Option<Credential>) -> Self {
        Self {
            channel,
            delegation,
        }
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn delegation(&self) -> Option<&Credential> {
        self.delegation.as_ref()
    }

    /// Estimated CBOR-encoded byte size of this Open's own new
    /// output -- same basis as `Channel::size_estimate`, but using
    /// this Open's own `delegation` (a base address costs more than
    /// an enterprise one) rather than assuming none.
    pub fn size_estimate(&self) -> usize {
        let address = crate::validator::address(
            cardano_sdk::NetworkId::from(cardano_sdk::Network::Preprod),
            self.delegation(),
        );
        let output = Output::new(address.into(), self.channel.buffered_value())
            .with_datum(crate::to_plutus_data(self.channel.datum()));
        output.to_cbor().len()
    }
}
