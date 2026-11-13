//! Our only behaviour so far: connectivity/totals, plus the same catalog route hit
//! every `pace` (`pace = Duration::ZERO` is the "firehose" case -- not a separate mode,
//! just this field's zero), plus an optional cutoff that stops a wave, resumable a
//! limited number of times via `next()`.
//!
//! No functions-as-config here -- hardcoded constants only (no variable
//! increment, no randomness). If/when a persona needs either, that's the
//! point to revisit this, not to pre-build it now.

use std::time::{Duration, Instant};

use super::fsm::{Action, Event, Fsm, Phase, Status, transition};

/// How long a rejected request (`InsufficientFunds`/`NoChannel`) makes this agent
/// wait before trying again -- applied on top of whatever `due_at` `pace` already
/// set, never instead of it. Independent of `pace` itself: at `pace = Duration::ZERO`
/// (every persona by default) there's otherwise nothing to stop a rejected request
/// from being retried on the very next `poll`, which is what a preprod run
/// showed as a sustained ~2.8k req/s flood of 402s against `subbit-gate` while a
/// channel's `Open` was still clearing `finality_depth` -- see
/// `canary-results.md`/this session's own history.
const REJECT_BACKOFF: Duration = Duration::from_secs(5);

/// A time-based cutoff: either a budget measured from when the current
/// wave started, or a fixed wall-clock deadline.
#[derive(Debug, Clone, Copy)]
pub enum TimeCutoff {
    Relative(Duration),
    Absolute(Instant),
}

/// Same shape as `TimeCutoff`, for the amount spent instead.
#[derive(Debug, Clone, Copy)]
pub enum AmountCutoff {
    Relative(u64),
    Absolute(u64),
}

pub struct Paced {
    phase: Phase,
    funds_total: u64,
    ious_total: u64,

    /// The one catalog route this agent always pays -- see `Action::Pay`'s own doc.
    level: char,
    pace: Duration,
    due_at: Option<Instant>,
    /// The `now` most recently handed to `poll` -- `handle_event` has no `now` of its
    /// own (see `Fsm::handle_event`'s signature), so a rejection's backoff is measured
    /// from here instead.
    last_poll_at: Instant,

    time_cutoff: Option<TimeCutoff>,
    amount_cutoff: Option<AmountCutoff>,
    /// Baseline the `Relative` cutoffs are measured against. Moved
    /// forward by `next()` -- that's the whole mechanism for "resumable."
    wave_started_at: Instant,
    wave_start_funds: u64,
    /// Set by `next()`, consumed by the following `poll` (which is when
    /// `now` is actually available to become the new `wave_started_at`).
    pending_restart: bool,
    /// How many more times `next()` may revive a cutoff-stopped wave.
    /// Once it hits 0, `next()` is a permanent no-op and the cutoff, once
    /// tripped, stays tripped forever.
    restarts_left: u32,
}

impl Paced {
    pub fn new(now: Instant, level: char, pace: Duration, restarts: u32) -> Self {
        Self {
            phase: Phase::Waiting,
            funds_total: 0,
            ious_total: 0,
            level,
            pace,
            due_at: None,
            last_poll_at: now,
            time_cutoff: None,
            amount_cutoff: None,
            wave_started_at: now,
            wave_start_funds: 0,
            pending_restart: false,
            restarts_left: restarts,
        }
    }

    pub fn with_time_cutoff(mut self, cutoff: TimeCutoff) -> Self {
        self.time_cutoff = Some(cutoff);
        self
    }

    pub fn with_amount_cutoff(mut self, cutoff: AmountCutoff) -> Self {
        self.amount_cutoff = Some(cutoff);
        self
    }

    /// OR across whichever cutoffs are set -- whichever is reached first
    /// stops the wave. No AND support: that's not a case we have yet.
    fn cutoff_reached(&self, now: Instant) -> bool {
        let time_hit = match self.time_cutoff {
            Some(TimeCutoff::Relative(budget)) => now >= self.wave_started_at + budget,
            Some(TimeCutoff::Absolute(deadline)) => now >= deadline,
            None => false,
        };
        let amount_hit = match self.amount_cutoff {
            Some(AmountCutoff::Relative(budget)) => {
                self.funds_total - self.wave_start_funds >= budget
            }
            Some(AmountCutoff::Absolute(threshold)) => self.funds_total >= threshold,
            None => false,
        };
        time_hit || amount_hit
    }
}

impl Fsm for Paced {
    fn poll(&mut self, now: Instant) -> Action {
        self.last_poll_at = now;
        if self.pending_restart {
            self.wave_started_at = now;
            self.wave_start_funds = self.funds_total;
            self.due_at = None;
            self.pending_restart = false;
        }
        if self.phase == Phase::End {
            return Action::Stop;
        }
        if self.cutoff_reached(now) {
            return Action::Stop;
        }
        match self.due_at {
            Some(due) if now < due => Action::Wait(due),
            _ => {
                if !self.pace.is_zero() {
                    self.due_at = Some(now + self.pace);
                }
                Action::Pay(self.level)
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Accepted(amount) => {
                self.funds_total = self.funds_total.saturating_add(amount);
                self.ious_total = self.ious_total.saturating_add(1);
            }
            // Retriable, but not for free: back off so a channel that isn't
            // spendable yet (pre-`Open`-finality, or a `Close` already landed on
            // gate's side but not this agent's) doesn't get hammered on every next
            // `poll` -- the only thing that otherwise stops that at `pace =
            // Duration::ZERO` is a cutoff, which most personas don't set.
            //
            // `Other` gets the same treatment: every rejection folded into it
            // (`OldIou`, `InvalidIou`, `OldAuth`, `InvalidAuth`, `RateLimited`,
            // `MutualNotSupported`) is just as retriable-but-not-free as
            // `InsufficientFunds`/`NoChannel` -- and x402's own legitimate
            // challenge/response handshake never reaches here at all, it's fully
            // absorbed inside `AnyStamp::spend`'s own retry-once logic (see
            // `plan-canary-l2-oldiou-backoff.md`'s TODO 2), so there's no
            // handshake case to carve out.
            Event::InsufficientFunds | Event::NoChannel | Event::Other => {
                let backoff_due = self.last_poll_at + REJECT_BACKOFF;
                self.due_at = Some(match self.due_at {
                    Some(due) => due.max(backoff_due),
                    None => backoff_due,
                });
            }
            Event::Inactive => {}
        }
        self.phase = transition(self.phase, event);
    }

    fn status(&self) -> Status {
        Status {
            phase: self.phase,
            funds_total: self.funds_total,
            ious_total: self.ious_total,
        }
    }

    fn next(&mut self) {
        if self.restarts_left > 0 {
            self.restarts_left -= 1;
            self.pending_restart = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consumer::driver::drive;

    /// Every test drives against this one fixed route; `respond` below echoes a fixed
    /// test amount (`10`) on acceptance, entirely decoupled from `level` itself -- the
    /// amount is never derived from the level, same as the `Agent` (see
    /// `fsm::Event::Accepted`'s own doc).
    const LEVEL: char = 'i';
    fn respond(_level: char) -> Event {
        Event::Accepted(10)
    }

    #[test]
    fn pace_zero_is_the_firehose_case_poll_never_waits() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0);
        let (pays, _) = drive(&mut f, t0, respond, 9);
        assert_eq!(pays, 9);
        assert_eq!(f.status().funds_total, 90);
    }

    #[test]
    fn a_rejection_backs_off_even_at_pace_zero() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0);
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::InsufficientFunds);
        // Unlike a `pace`-driven `Wait`, this one isn't optional: at `pace =
        // Duration::ZERO` there's nothing else that would ever stop an immediate
        // retry on the very next `poll`.
        assert_eq!(f.poll(t0), Action::Wait(t0 + REJECT_BACKOFF));
        assert_eq!(f.poll(t0 + REJECT_BACKOFF), Action::Pay(LEVEL));
    }

    #[test]
    fn an_other_rejection_backs_off_even_at_pace_zero() {
        // Mirrors `a_rejection_backs_off_even_at_pace_zero`, but for the
        // catch-all `Event::Other` (what `OldIou`, `InvalidIou`, `OldAuth`,
        // `InvalidAuth`, `RateLimited`, `MutualNotSupported` all classify
        // into, per `fsm::Event::from`) -- this is the regression test for
        // the firehose-throughput bug: `Other` used to fall through to a
        // no-op, so an agent retrying a stale `OldIou` spend at `pace =
        // Duration::ZERO` would spin at full speed forever instead of
        // backing off.
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0);
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Other);
        assert_eq!(f.poll(t0), Action::Wait(t0 + REJECT_BACKOFF));
        assert_eq!(f.poll(t0 + REJECT_BACKOFF), Action::Pay(LEVEL));
    }

    #[test]
    fn a_rejection_backoff_never_shortens_an_already_longer_pace_wait() {
        let t0 = Instant::now();
        let pace = REJECT_BACKOFF * 4;
        let mut f = Paced::new(t0, LEVEL, pace, 0);
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        // `poll` already staged `due_at = t0 + pace` for the next dispatch (`pace`
        // governs spacing between attempts regardless of outcome); a rejection must
        // not pull that back in to the shorter backoff.
        f.handle_event(Event::NoChannel);
        assert_eq!(f.poll(t0), Action::Wait(t0 + pace));
    }

    #[test]
    fn a_positive_pace_alternates_wait_and_pay() {
        let t0 = Instant::now();
        let pace = Duration::from_millis(5);
        let mut f = Paced::new(t0, LEVEL, pace, 0);
        // 5 driver steps = Pay, Wait, Pay, Wait, Pay -- 3 pays, 2 waits.
        let (pays, now) = drive(&mut f, t0, respond, 5);
        assert_eq!(pays, 3);
        assert_eq!(f.status().funds_total, 30);
        assert_eq!(now, t0 + pace * 2);
    }

    #[test]
    fn a_relative_amount_cutoff_stops_the_wave() {
        let t0 = Instant::now();
        let mut f =
            Paced::new(t0, LEVEL, Duration::ZERO, 0).with_amount_cutoff(AmountCutoff::Relative(25));
        let (pays, _) = drive(&mut f, t0, respond, 10);
        assert_eq!(pays, 3); // 10, 10, 10 -- cutoff checked before paying, not clamped
        assert_eq!(f.status().funds_total, 30);
    }

    #[test]
    fn an_absolute_time_cutoff_stops_the_wave() {
        let t0 = Instant::now();
        let deadline = t0 + Duration::from_millis(1);
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0)
            .with_time_cutoff(TimeCutoff::Absolute(deadline));
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.poll(deadline), Action::Stop);
    }

    #[test]
    fn a_relative_time_cutoff_stops_the_wave_after_a_budget_elapses() {
        let t0 = Instant::now();
        let budget = Duration::from_millis(10);
        let mut f =
            Paced::new(t0, LEVEL, Duration::ZERO, 0).with_time_cutoff(TimeCutoff::Relative(budget));
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.poll(t0 + budget), Action::Stop);
    }

    #[test]
    fn an_absolute_amount_cutoff_stops_the_wave_at_the_lifetime_total() {
        let t0 = Instant::now();
        let mut f =
            Paced::new(t0, LEVEL, Duration::ZERO, 0).with_amount_cutoff(AmountCutoff::Absolute(20));
        let (pays, _) = drive(&mut f, t0, respond, 10);
        assert_eq!(pays, 2);
        assert_eq!(f.status().funds_total, 20);
    }

    #[test]
    fn next_is_a_no_op_with_zero_restarts() {
        let t0 = Instant::now();
        let mut f =
            Paced::new(t0, LEVEL, Duration::ZERO, 0).with_amount_cutoff(AmountCutoff::Relative(10));
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.poll(t0), Action::Stop);
        f.next();
        assert_eq!(f.poll(t0), Action::Stop); // no restarts left, stays stopped
    }

    #[test]
    fn next_restarts_a_cutoff_wave_up_to_the_allowed_count() {
        let t0 = Instant::now();
        let mut f =
            Paced::new(t0, LEVEL, Duration::ZERO, 2).with_amount_cutoff(AmountCutoff::Relative(10));

        // Wave 1.
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.poll(t0), Action::Stop);

        // Restart 1 of 2.
        f.next();
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.status().funds_total, 20); // lifetime total keeps accumulating
        assert_eq!(f.poll(t0), Action::Stop);

        // Restart 2 of 2.
        f.next();
        assert_eq!(f.poll(t0), Action::Pay(LEVEL));
        f.handle_event(Event::Accepted(10));
        assert_eq!(f.poll(t0), Action::Stop);

        // Restarts exhausted: next() is now permanently a no-op.
        f.next();
        assert_eq!(f.poll(t0), Action::Stop);
    }

    #[test]
    fn inactive_stops_permanently_regardless_of_restarts_left() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 5);
        f.handle_event(Event::Inactive);
        assert_eq!(f.poll(t0), Action::Stop);
        f.next();
        assert_eq!(f.poll(t0), Action::Stop);
    }
}
