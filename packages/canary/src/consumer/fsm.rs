//! Sans-io. `Fsm` is the minimal trait the driver knows about -- it makes
//! no assumption about behaviour at all. `Paced` (in `paced.rs`) is
//! currently the only impl.

use std::time::Instant;

use subbit_l2_dto::Error as CoreError;

/// The outcome of one dispatch. `Accepted`'s amount is the observed delta in the
/// server-confirmed committed total (see `Agent::classified_spend`) -- never a value
/// this crate invents or echoes back itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Accepted(u64),
    /// Retriable -- an `Add` landing fixes it.
    InsufficientFunds,
    /// Terminal: gate observed the on-chain `Close`.
    Inactive,
    /// Not indexed yet. Retriable.
    NoChannel,
    Other,
}

impl From<CoreError> for Event {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::InsufficientFunds => Event::InsufficientFunds,
            CoreError::Inactive => Event::Inactive,
            CoreError::NoChannel => Event::NoChannel,
            _ => Event::Other,
        }
    }
}

/// Deliberately *not* one variant per rejection cause -- `NoChannel` and a
/// post-rejection backoff are different reasons to be waiting, but
/// identical in every way any impl acts on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Running,
    /// Terminal, sticky.
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Fire one spend request against this catalog route (e.g. `'i'` for
    /// `roman/i`) -- the FSM picks *which* route, never an amount; the
    /// cost is whatever the shared catalog (`crate::catalog`) declares for
    /// it, resolved independently by both sides. See `Event::Accepted`'s
    /// own doc.
    Pay(char),
    /// Call `poll` again no earlier than this instant.
    Wait(Instant),
    Stop,
}

/// What `Orchestrator` may read off a spawned agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    pub funds_total: u64,
    pub ious_total: u64,
}

/// Everything driven in this system implements this. `next()` defaults to
/// a no-op. It arrives independent of any dispatch, never through
/// `handle_event` -- see `paced.rs`'s own history for why.
pub trait Fsm {
    fn poll(&mut self, now: Instant) -> Action;
    fn handle_event(&mut self, event: Event);
    fn status(&self) -> Status;

    fn next(&mut self) {}
}

/// | from       | event               | to        |
/// |------------|---------------------|-----------|
/// | any        | `Inactive`          | `End`     |
/// | `Waiting`  | `Accepted`          | `Running` |
/// | `Running`  | `InsufficientFunds` | `Waiting` |
/// | `End`      | *(anything)*        | `End`     |
///
/// Everything else not listed leaves the phase where it was.
pub(crate) fn transition(phase: Phase, event: Event) -> Phase {
    if matches!(event, Event::Inactive) {
        return Phase::End;
    }
    match (phase, event) {
        (Phase::End, _) => Phase::End,
        (Phase::Waiting, Event::Accepted(_)) => Phase::Running,
        (Phase::Running, Event::InsufficientFunds) => Phase::Waiting,
        (unchanged, _) => unchanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_moves_waiting_to_running_on_accepted() {
        assert_eq!(
            transition(Phase::Waiting, Event::Accepted(5)),
            Phase::Running
        );
    }

    #[test]
    fn transition_moves_running_to_waiting_on_insufficient_funds() {
        assert_eq!(
            transition(Phase::Running, Event::InsufficientFunds),
            Phase::Waiting
        );
    }

    #[test]
    fn transition_to_end_on_inactive_is_sticky() {
        for phase in [Phase::Waiting, Phase::Running, Phase::End] {
            assert_eq!(transition(phase, Event::Inactive), Phase::End);
        }
        assert_eq!(transition(Phase::End, Event::Accepted(1)), Phase::End);
    }

    #[test]
    fn core_error_classifies_into_the_matching_event() {
        assert_eq!(
            Event::from(CoreError::InsufficientFunds),
            Event::InsufficientFunds
        );
        assert_eq!(Event::from(CoreError::Inactive), Event::Inactive);
        assert_eq!(Event::from(CoreError::NoChannel), Event::NoChannel);
        assert_eq!(Event::from(CoreError::OldAuth), Event::Other);
        assert_eq!(Event::from(CoreError::RateLimited), Event::Other);
    }
}
