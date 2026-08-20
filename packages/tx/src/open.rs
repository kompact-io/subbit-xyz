use cardano_sdk::Credential;

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
}
