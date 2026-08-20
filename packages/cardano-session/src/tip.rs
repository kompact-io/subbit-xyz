use std::collections::BTreeMap;

use cardano_sdk::{Address, Input, Output, address::kind::Shelley};
use serde::{Deserialize, Serialize};

/// Cache of UTXOs at addresses beyond the wallet's own. Never makes a
/// network call itself - `Session` fetches and feeds it results. "Pure"
/// only in that sense (no I/O); it's ordinary `&mut self` state otherwise.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tip {
    utxos: BTreeMap<Address<Shelley>, BTreeMap<Input, Output>>,
}

impl Tip {
    pub fn empty() -> Self {
        Self::default()
    }

    /// `None` means never fetched, not "fetched and empty".
    pub fn utxos_at(&self, address: &Address<Shelley>) -> Option<&BTreeMap<Input, Output>> {
        self.utxos.get(address)
    }

    pub fn addresses(&self) -> impl Iterator<Item = &Address<Shelley>> {
        self.utxos.keys()
    }

    pub fn is_tracked(&self, address: &Address<Shelley>) -> bool {
        self.utxos.contains_key(address)
    }

    /// Records a fresh snapshot for one address, replacing whatever was
    /// cached before (and starting to track it, if it wasn't already).
    pub fn refresh(&mut self, address: Address<Shelley>, utxos: BTreeMap<Input, Output>) {
        self.utxos.insert(address.clone(), utxos);
    }

    pub fn refresh_many(
        &mut self,
        snapshots: impl IntoIterator<Item = (Address<Shelley>, BTreeMap<Input, Output>)>,
    ) {
        self.utxos.extend(snapshots);
    }

    /// Unlike caching an empty result, this stops `refresh_all` from
    /// touching `address` again.
    pub fn untrack(&mut self, address: &Address<Shelley>) {
        self.utxos.remove(address);
    }

    pub fn clear(&mut self) {
        self.utxos.clear();
    }
}
