//! The L2 agent: an HTTP/stamp-driven spend loop for one consumer's own lifecycle,
//! entirely self-contained. It holds an [`AnyStamp`](super::any_stamp::AnyStamp) (the
//! whole reqwest boundary, one dialect, including the identity material needed to
//! sign IOUs -- see that module's own doc); it never reads the seat's `L1` state and is
//! never read by it directly. `super::Consumer` is the only thing that knows a given
//! `L1` and `Agent` belong to the same seat -- it builds this agent (`init`), spawns its
//! spend loop and holds the resulting `JoinHandle` itself, and is the only path by which
//! anything reaches `status`/`next`/`tidy` on it.
//!
//! `Agent` itself never calls `tokio::spawn` -- `init` connects and builds this agent's
//! FSM, but spawns nothing; running it concurrently is `Consumer::new`'s job, and the
//! `JoinHandle` it gets back is `Consumer`'s to hold, not this type's.
//!
//! The FSM (`Paced`) is this agent's own private state -- never returned from `init`,
//! never a parameter of `run`. It lives behind a `tokio::sync::Mutex` (not `std`'s:
//! `run` holds the lock across `.await` points for its own entire lifetime, which an
//! async-aware mutex is built for) rather than `AnyStamp`/`status`/`notify` getting a
//! sibling field on `Agent` that a caller could reach in and hand around.
//!
//! No field here is individually `Arc`'d either. `Agent` is only ever handled as
//! `Arc<Agent>` (that's what `init` hands back) -- cloning that one outer `Arc`
//! already shares everything inside it between `Orchestrator`'s own handle and the
//! task `run` is spawned as. A second, per-field `Arc` on top of that would just be
//! sharing something already shared.

use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::any_stamp::{AnyStamp, SpendError};
use super::driver;
use super::fsm::{self, Event, Fsm};
use super::paced::{AmountCutoff, Paced};
use super::persona;

/// Everything this seat's `Agent` is fetched/built from, when it has one at all.
pub struct Config {
    pub provider_base_url: String,
    pub behavior: persona::Behavior,
}

pub(super) struct Agent {
    stamp: AnyStamp,
    fsm: tokio::sync::Mutex<Paced>,
    status: Mutex<fsm::Status>,
    notify: Notify,
}

/// Everything about this agent that actually changes over its lifetime -- the one
/// thing `Consumer::status` wants, so it's what `Agent::status` returns directly,
/// rather than making the caller stitch it together from several narrower getters.
/// `dialect` is deliberately not here: it's fixed at `init` and never changes, so it
/// belongs with the rest of this seat's fixed `Settings`, not a per-call status read.
pub struct Status {
    pub phase: fsm::Phase,
    pub funds_total: u64,
    pub ious_total: u64,
    pub spent: u64,
    pub committed: Option<u64>,
    pub committed_iou: Option<subbit_core::Iou>,
}

impl Agent {
    /// Connects this agent's `AnyStamp` (`AnyStamp::init` does the `/v1/terms`
    /// fetch) and builds its own FSM -- spawns nothing. See this module's own doc
    /// comment on why neither is ever exposed past this constructor.
    pub async fn init(
        seed: [u8; 32],
        tag: impl Into<Vec<u8>>,
        config: Config,
    ) -> anyhow::Result<Arc<Self>> {
        let tag: Vec<u8> = tag.into();
        let Config {
            provider_base_url,
            behavior,
        } = config;
        let stamp = AnyStamp::init(&seed, &tag, behavior.dialect, provider_base_url).await?;

        let mut fsm = Paced::new(
            std::time::Instant::now(),
            behavior.level,
            behavior.pace,
            u32::MAX,
        );
        if let Some(cutoff) = behavior.amount_cutoff {
            fsm = fsm.with_amount_cutoff(AmountCutoff::Absolute(cutoff));
        }
        let status = Mutex::new(fsm.status());
        let notify = Notify::new();

        Ok(Arc::new(Self {
            stamp,
            fsm: tokio::sync::Mutex::new(fsm),
            status,
            notify,
        }))
    }

    /// This agent's own spend loop -- the whole FSM-driving boundary, see
    /// `driver::drive_async`'s own doc for why it never returns on its own (there's no
    /// reason to await this directly: `Orchestrator` spawns it and holds the
    /// `JoinHandle`, see this module's own doc). `AnyStamp::spend` returns its own,
    /// FSM-agnostic `SpendError` (see that module's own doc) -- matching it into this
    /// agent's own `fsm::Event` vocabulary is this agent's job, not `AnyStamp`'s.
    pub async fn run(self: Arc<Self>) {
        let mut fsm = self.fsm.lock().await;
        driver::drive_async(
            &mut *fsm,
            |level| {
                let agent = Arc::clone(&self);
                async move {
                    match agent.stamp.spend(level).await {
                        Ok(amount) => Event::Accepted(amount),
                        Err(SpendError::Protocol(core)) => Event::from(core),
                        Err(SpendError::Other) => Event::Other,
                    }
                }
            },
            |s| *self.status.lock().unwrap() = s,
            &self.notify,
        )
        .await;
    }

    /// Everything about this agent that changes over time, in one read. The FSM's own
    /// `phase`/`funds_total`/`ious_total` come from the cached snapshot `run` updates;
    /// `spent`/`committed`/`committed_iou` delegate straight to `AnyStamp`.
    pub fn status(&self) -> Status {
        let s = *self.status.lock().unwrap();
        Status {
            phase: s.phase,
            funds_total: s.funds_total,
            ious_total: s.ious_total,
            spent: self.stamp.cache().spent(),
            committed: self.stamp.committed(),
            committed_iou: self.stamp.committed_iou(),
        }
    }

    /// Nudges this agent to resume a cutoff-stopped wave.
    pub fn next(&self) {
        self.notify.notify_one();
    }
}
