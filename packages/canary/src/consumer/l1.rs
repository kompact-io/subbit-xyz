//! `L1`: one roster seat's own on-chain `Add`/`Close` schedule, sampled once at
//! persona-generation time. Private to `super` (the `consumer` module) -- see its own
//! doc comment on why `Consumer` is the only thing that touches it. `want` reads only
//! on-chain `Can` state and the `Round` the orchestrator hands it, never an `Agent`'s
//! progress, and holds no clock of its own.

use subbit_core::{Duration, Instant};
use subbit_tx::step::{Can, Want};

use crate::time::Round;

pub(super) struct L1 {
    add_amounts: Vec<u64>,
    /// How far past the earliest legal `Elapse` this seat waits. The
    /// canary default is deliberately conservative; the claim-race scenario
    /// can shrink it to drive `Elapse` as early as the chain allows.
    elapse_margin: Duration,
}

impl L1 {
    #[cfg(test)]
    pub(super) fn new(add_amounts: Vec<u64>) -> Self {
        Self::with_elapse_margin(add_amounts, Duration::from_secs(120))
    }

    pub(super) fn with_elapse_margin(add_amounts: Vec<u64>, elapse_margin: Duration) -> Self {
        Self {
            add_amounts,
            elapse_margin,
        }
    }

    /// This seat's own decision for `round` -- a pure function of `(round, can)`, no
    /// mutable state. `add_amounts` fires strictly by round index; once exhausted (by
    /// running out of scheduled `Add`s, or by being told `Round::Tidying`), heads for
    /// `Close`, then -- standard behaviour, not an opt-in -- straight on to `Elapse`
    /// once that `Close` lands.
    pub(super) fn want(&self, round: Round, can: &[Can], close_upper: Instant) -> Option<Want> {
        if can.contains(&Can::End) {
            return Some(Want::End);
        }
        match round {
            Round::Tidying => self.exit_want(can, close_upper),
            Round::Active(r) => match self.add_amounts.get(r as usize) {
                Some(&amount) => can.contains(&Can::Add).then_some(Want::Add { amount }),
                // Exhausted this seat's own Add budget -- same exit behaviour as
                // Tidying, just reached by running out rather than being told.
                None => self.exit_want(can, close_upper),
            },
        }
    }

    /// `Close` while `Opened`; once `Closed`, `Elapse`. Shared by both the
    /// "ran out of scheduled Adds" and the explicit `Tidying` paths -- same exit,
    /// two ways to reach it.
    fn exit_want(&self, can: &[Can], close_upper: Instant) -> Option<Want> {
        for c in can {
            match c {
                Can::Close => return Some(Want::Close { upper: close_upper }),
                // 120s by default: 1s/20s both measured too tight live (rejected
                // submissions); matches close_upper's own margin for the same reason.
                // Tests can shrink it to race claim at the earliest legal slot.
                Can::Elapse { after } => {
                    return Some(Want::Elapse {
                        lower: *after + self.elapse_margin,
                    });
                }
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(secs: u64) -> Instant {
        Instant::from_millis(secs * 1000)
    }

    #[test]
    fn end_wins_regardless_of_round() {
        let l1 = L1::new(vec![10]);
        for round in [Round::Active(0), Round::Tidying] {
            assert_eq!(l1.want(round, &[Can::End], instant(0)), Some(Want::End));
        }
    }

    #[test]
    fn active_round_wants_its_own_scheduled_add() {
        let l1 = L1::new(vec![10, 20]);
        assert_eq!(
            l1.want(Round::Active(1), &[Can::Add], instant(0)),
            Some(Want::Add { amount: 20 })
        );
    }

    #[test]
    fn active_round_wants_nothing_if_chain_cant_add_yet() {
        let l1 = L1::new(vec![10]);
        assert_eq!(
            l1.want(Round::Active(0), &[Can::Sub { available: 5 }], instant(0)),
            None
        );
    }

    #[test]
    fn exhausting_the_add_schedule_heads_for_close() {
        let l1 = L1::new(vec![10]);
        let upper = instant(100);
        assert_eq!(
            l1.want(Round::Active(1), &[Can::Close], upper),
            Some(Want::Close { upper })
        );
    }

    #[test]
    fn tidying_heads_for_close_even_mid_schedule() {
        let l1 = L1::new(vec![10, 20, 30]);
        let upper = instant(100);
        assert_eq!(
            l1.want(Round::Tidying, &[Can::Close], upper),
            Some(Want::Close { upper })
        );
    }

    #[test]
    fn exit_elapses_120s_past_the_close_period_once_closed() {
        let l1 = L1::new(vec![]);
        let after = instant(50);
        assert_eq!(
            l1.want(Round::Tidying, &[Can::Elapse { after }], instant(0)),
            Some(Want::Elapse {
                lower: after + Duration::from_secs(120)
            })
        );
    }

    #[test]
    fn elapse_margin_is_configurable_for_the_claim_race() {
        let l1 = L1::with_elapse_margin(vec![], Duration::from_secs(1));
        let after = instant(50);
        assert_eq!(
            l1.want(Round::Tidying, &[Can::Elapse { after }], instant(0)),
            Some(Want::Elapse {
                lower: after + Duration::from_secs(1)
            })
        );
    }

    #[test]
    fn no_actionable_can_wants_nothing() {
        let l1 = L1::new(vec![]);
        assert_eq!(
            l1.want(Round::Tidying, &[Can::Sub { available: 5 }], instant(0)),
            None
        );
    }
}
