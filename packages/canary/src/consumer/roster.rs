//! Turns a `--persona` roster into everything `Orchestrator::build` needs
//! to register signing keys and build real `Consumer`s: identity/tag
//! derivation and persona sampling are consumer-construction concerns in
//! their own right, independent of chain or network state.

use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::identity::{self, Identity};

use super::persona::{self, Behavior};

fn tag_for(l2_seed: &[u8; 32], i: usize) -> Vec<u8> {
    identity::tag_for(l2_seed, "", i)
}

/// Consumer index `i`'s identity material, derived the same way whether
/// it ends up persona-driven (`plan_roster`) or a bare placeholder (a
/// teardown scan).
pub(crate) fn identity_for(l2_seed: &[u8; 32], i: usize) -> (Identity, [u8; 32], Vec<u8>) {
    let seed = identity::consumer_seed(l2_seed, i);
    let tag = tag_for(l2_seed, i);
    (Identity::from_seed(seed), seed, tag)
}

/// Seeds the one `StdRng` every persona samples from, so a given
/// `L2_SEED_HEX` reproduces the exact same sampled behaviors, not just
/// the same identities.
fn rng_from_seed(l2_seed: &[u8; 32]) -> StdRng {
    let derived = identity::derive_seed(l2_seed, "persona-rng");
    StdRng::seed_from_u64(u64::from_le_bytes(derived[..8].try_into().unwrap()))
}

/// One roster seat, fully planned: identity, tag, and a sampled
/// `Behavior`, ready for `ConsumerBuilder`.
pub(crate) struct PlannedConsumer {
    pub identity: Identity,
    pub seed: [u8; 32],
    pub tag: Vec<u8>,
    pub persona_name: String,
    pub behavior: Behavior,
}

/// Parses `roster_specs`, expands it into one seat per consumer, and
/// samples each one's own concrete `Behavior` -- everything
/// `Orchestrator::build` needs, independent of chain or network state.
pub(crate) fn plan_roster(
    l2_seed: &[u8; 32],
    roster_specs: &[String],
) -> anyhow::Result<Vec<PlannedConsumer>> {
    let roster = persona::parse_roster(roster_specs)?;
    anyhow::ensure!(
        roster.iter().map(|e| e.count).sum::<usize>() > 0,
        "persona roster has zero total consumers"
    );

    let personas: Vec<&persona::Persona> = roster
        .iter()
        .flat_map(|entry| std::iter::repeat(&entry.persona).take(entry.count))
        .collect();

    let mut rng = rng_from_seed(l2_seed);
    personas
        .into_iter()
        .enumerate()
        .map(|(i, persona)| {
            let (identity, seed, tag) = identity_for(l2_seed, i);
            let behavior = persona.generate(&mut rng);
            println!(
                "canary: [{i}] {} -- dialect={:?} deposit={} adds={:?} spend_counts={:?}",
                persona.name,
                behavior.dialect,
                behavior.deposit,
                behavior.add_amounts,
                behavior.spend_counts
            );
            Ok(PlannedConsumer {
                identity,
                seed,
                tag,
                persona_name: persona.name.clone(),
                behavior,
            })
        })
        .collect()
}
