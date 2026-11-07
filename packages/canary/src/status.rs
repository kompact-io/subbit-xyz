//! Read-only census of the validator address, split by `Stage`.

use std::collections::BTreeMap;

use anyhow::Context;
use cardano_sdk::Input;
use subbit_core::Hash28;
use subbit_l1_dto::Stage;

use crate::chain::Chain;
use crate::identity;

pub async fn run(l2_seed: [u8; 32], scan_consumers: usize, mut chain: Chain) -> anyhow::Result<()> {
    let seeds = known_seeds(&l2_seed, scan_consumers);
    let identity_to_seed = identity_to_seed_map(&seeds);

    let recognition = recognize_channels(&mut chain, &identity_to_seed).await?;

    let mut opened = 0;
    let mut closed = 0;
    let mut settled = 0;
    for (_, channel, _) in &recognition.recognized {
        match channel.variables().stage() {
            Stage::Opened { .. } => opened += 1,
            Stage::Closed { .. } => closed += 1,
            Stage::Settled => settled += 1,
        }
    }

    println!(
        "canary status: {} utxo(s) at the validator address",
        recognition.total
    );
    println!(
        "canary status: {} belong to this run's keys ({} unrecognized -- not this run's, or \
         --scan-consumers {scan_consumers} is too small to cover them)",
        recognition.recognized.len(),
        recognition.unrecognized_count()
    );
    println!("canary status:   Opened:  {opened}");
    println!("canary status:   Closed:  {closed}");
    println!("canary status:   Settled: {settled}");

    Ok(())
}

/// Every consumer/provider identity this run is willing to recognize, up to `scan_consumers`.
fn known_seeds(l2_seed: &[u8; 32], scan_consumers: usize) -> Vec<[u8; 32]> {
    let mut seeds = vec![identity::provider_seed(l2_seed)];
    for i in 0..scan_consumers {
        seeds.push(identity::consumer_seed(l2_seed, i));
    }
    seeds
}

fn identity_to_seed_map(seeds: &[[u8; 32]]) -> BTreeMap<Hash28, [u8; 32]> {
    seeds
        .iter()
        .map(|s| (identity::hash28_from_seed(s), *s))
        .collect()
}

/// Every UTxO at the validator address, split into recognized (ours) vs. not.
struct Recognition {
    total: usize,
    recognized: Vec<(Input, subbit_tx::Channel, [u8; 32])>,
}

impl Recognition {
    fn unrecognized_count(&self) -> usize {
        self.total - self.recognized.len()
    }
}

async fn recognize_channels(
    chain: &mut Chain,
    identity_to_seed: &BTreeMap<Hash28, [u8; 32]>,
) -> anyhow::Result<Recognition> {
    chain
        .refresh()
        .await
        .context("refreshing tracked channels")?;

    let utxos = chain.channels();
    let total = utxos.len();
    let recognized = utxos
        .iter()
        .filter_map(|(input, channel)| {
            let seed = *identity_to_seed.get(channel.constants().consumer())?;
            Some((input.clone(), channel.clone(), seed))
        })
        .collect();

    Ok(Recognition { total, recognized })
}
