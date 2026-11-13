use std::sync::Arc;

use subbit_core::{Duration, Hash28, Instant, Tag, VerifyingKey};
use subbit_tx::step::{Can, Want};

use crate::identity::hash28_from_seed;
use crate::time::Round;

mod agent;
mod any_stamp;
mod driver;
mod fsm;
mod l1;
mod paced;
mod persona;
mod roster;

pub use agent::{Config as AgentConfig, Status as AgentStatus};
pub use any_stamp::Dialect;
pub(crate) use roster::{identity_for, plan_roster};

use agent::Agent;
use l1::L1;

// FIXME : move to ./consumer/identity.rs
pub struct Identity {
    pub tag: Tag,
    pub vkh: Hash28,
    pub iou_key: VerifyingKey,
    signing_key: ed25519_dalek::SigningKey,
}

impl Identity {
    fn derive(seed: [u8; 32], tag: Tag) -> Self {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        Self {
            vkh: hash28_from_seed(&seed),
            iou_key: VerifyingKey::from(signing_key.verifying_key().to_bytes()),
            tag,
            signing_key,
        }
    }

    /// Signs an IOU for `amount` entirely offline -- no live `Agent`/gate
    /// connection needed, just this consumer's own key. Used by admin-
    /// forced `Settle` (`Orchestrator`'s `admin_settle`) as a fallback
    /// when there's no connected `Agent` to supply a live
    /// `committed_iou` (a `tidy` placeholder consumer never has one):
    /// signing `amount` equal to the channel's own already-on-chain
    /// `subbed` is a legitimate zero-gain `Settle` (`Variables::settle`
    /// accepts an unchanged amount as a `Closed -> Settled`
    /// transition in its own right), keeping that pathway available
    /// instead of silently falling through to the consumer's own
    /// unilateral `Elapse`.
    pub fn sign_iou(&self, amount: u64) -> subbit_core::Iou {
        subbit_stamp::Id::new(self.signing_key.clone(), self.tag.clone()).iou(amount)
    }

    pub fn keytag(&self) -> String {
        self.iou_key.to_string() + &self.tag.to_string()
    }
}

pub struct Settings {
    pub identity: Identity,
    pub deposit: u64,
    pub dialect: Option<Dialect>,
}

pub struct Consumer {
    settings: Settings,
    l1: L1,
    agent: Option<Arc<Agent>>,
    agent_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Consumer {
    pub async fn init(
        seed: [u8; 32],
        tag: Tag,
        agent: Option<AgentConfig>,
        elapse_margin: Duration,
    ) -> anyhow::Result<Self> {
        let identity = Identity::derive(seed, tag.clone());
        let (deposit, dialect, add_amounts, agent) = match agent {
            Some(config) => {
                let deposit = config.behavior.deposit;
                let dialect = config.behavior.dialect;
                let add_amounts = config.behavior.add_amounts.clone();
                let agent = Agent::init(seed, tag, config).await?;
                (deposit, Some(dialect), add_amounts, Some(agent))
            }
            None => (0, None, Vec::new(), None),
        };
        let (agent, agent_handle) = match agent {
            Some(a) => {
                let handle = tokio::spawn(Arc::clone(&a).run());
                (Some(a), Some(handle))
            }
            None => (None, None),
        };
        Ok(Self {
            settings: Settings {
                identity,
                deposit,
                dialect,
            },
            l1: L1::with_elapse_margin(add_amounts, elapse_margin),
            agent,
            agent_handle,
        })
    }

    /// This seat's fixed identity/deposit -- never mutated past construction.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// This seat's on-chain decision for `round`, given the chain's own `can` for it.
    /// `L1` holds no clock of its own -- `round` is the one thing `Orchestrator`
    /// drives both this and its own `next()`/kill/tidy sequencing by.
    pub fn run(&self, round: Round, can: &[Can], close_upper: Instant) -> Option<Want> {
        self.l1.want(round, can, close_upper)
    }

    /// Nudges this seat's `Agent` to resume a cutoff-stopped wave. No-op with no `Agent`.
    pub fn next(&self) {
        if let Some(agent) = &self.agent {
            agent.next();
        }
    }

    /// This seat's current off-chain status, drilling down into its `Agent` -- `None` for
    /// a seat with no `Agent` (a `tidy` placeholder).
    pub fn status(&self) -> Option<AgentStatus> {
        self.agent.as_ref().map(|a| a.status())
    }

    /// Aborts this seat's spawned `Agent` task only -- touches nothing on `L1`. Called
    /// once `Orchestrator` exceeds `max_rounds`, right before it switches every
    /// consumer over to `Round::Tidying`.
    pub fn kill_agent(&self) {
        if let Some(handle) = &self.agent_handle {
            handle.abort();
        }
    }
}
