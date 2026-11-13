//! The wiring layer: this is where `tokio` (or `rayon`, or whatever)
//! lives. It knows *nothing* about behaviour -- no pacing, no
//! amounts, no thresholds. It only ever translates `Action` into work
//! (sleep until an instant, fire an actual request) and the outcome
//! back into `Event`. Every decision about *how much*/*how paced* already
//! happened inside whichever `impl Fsm` is being driven -- by the time
//! `drive` sees `Action::Pay(amount)`, `amount` is final.
//!
//! `drive` here is synchronous and deterministic (a plain callback instead
//! of I/O), which is what makes it usable for both examples and tests
//! without a runtime. An async driver is the same shape with `respond`
//! an actual spend request and `Action::Wait` a `tokio::time::sleep`.
//!
//! `max_steps` is a safety net against an unbounded behaviour with no
//! cutoff of its own -- `drive` has no opinion on when to stop beyond
//! whatever `Action::Stop` the `Fsm` itself produces.

use std::future::Future;
use std::time::Instant;

use tokio::sync::Notify;

use super::fsm::{Action, Event, Fsm, Status};

pub fn drive(
    f: &mut impl Fsm,
    start: Instant,
    mut respond: impl FnMut(char) -> Event,
    max_steps: usize,
) -> (usize, Instant) {
    let mut now = start;
    let mut pays = 0;
    for _ in 0..max_steps {
        match f.poll(now) {
            Action::Stop => return (pays, now),
            Action::Wait(due) => now = due,
            Action::Pay(level) => {
                f.handle_event(respond(level));
                pays += 1;
            }
        }
    }
    (pays, now)
}

/// The async counterpart promised by this module's own doc comment: same shape as
/// `drive`, but `respond` is a spend request and `Action::Wait` a
/// `tokio::time::sleep`. `respond`/`on_status` are the only places behaviour-specific
/// code (stamp/HTTP, a shared `Status` cell) ever touches this loop -- this function
/// itself still knows nothing about pacing, amounts, or thresholds.
///
/// Loops forever: `Action::Stop` parks on `notify` instead of returning, so a later
/// `notify.notify_one()` (`Fsm::next()`'s own trigger -- e.g. `Consumer::agent_next`)
/// can revive a cutoff-stopped wave without the caller needing to respawn this task.
/// There is no other way out of this loop -- see `fsm::Fsm`'s own doc on the kill-only
/// teardown contract; a caller stops it by aborting the task driving it.
pub async fn drive_async<F, R, Fut>(
    f: &mut F,
    mut respond: R,
    on_status: impl Fn(Status),
    notify: &Notify,
) where
    F: Fsm,
    R: FnMut(char) -> Fut,
    Fut: Future<Output = Event>,
{
    loop {
        let now = Instant::now();
        match f.poll(now) {
            Action::Stop => {
                on_status(f.status());
                notify.notified().await;
                f.next();
            }
            Action::Wait(due) => {
                let dur = due.saturating_duration_since(now);
                tokio::select! {
                    () = tokio::time::sleep(dur) => {}
                    () = notify.notified() => f.next(),
                }
            }
            Action::Pay(level) => {
                let event = respond(level).await;
                f.handle_event(event);
                on_status(f.status());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::consumer::fsm::Phase;
    use crate::consumer::paced::Paced;

    const LEVEL: char = 'i';

    #[test]
    fn drives_a_flat_out_paced_fsm_with_whatever_respond_decides() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0);
        let (pays, _) = drive(&mut f, t0, |_level| Event::Accepted(10), 5);
        assert_eq!(pays, 5);
        assert_eq!(f.status().funds_total, 50);
    }

    #[test]
    fn retries_resolve_through_respond_same_as_any_other_event() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0);
        let mut attempts = 0;
        // Each rejection now also costs one `Wait` step (the rejection backoff --
        // see `paced::REJECT_BACKOFF`), so 4 pays takes 7 driver steps (4 pays + 3
        // intervening waits), not 4.
        let (pays, _) = drive(
            &mut f,
            t0,
            |_| {
                attempts += 1;
                if attempts < 4 {
                    Event::NoChannel
                } else {
                    Event::Accepted(1)
                }
            },
            7,
        );
        assert_eq!(pays, 4);
        assert_eq!(f.status().phase, Phase::Running);
        assert_eq!(f.status().funds_total, 1);
    }

    /// `drive_async` parks on `Action::Stop` instead of returning -- confirmed here by
    /// racing it against a timeout: it must still be running (the cutoff tripped,
    /// nothing ever notifies it) when the timeout wins.
    #[tokio::test]
    async fn drive_async_parks_on_stop_instead_of_returning() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 0)
            .with_amount_cutoff(crate::consumer::paced::AmountCutoff::Absolute(10));
        let notify = Notify::new();
        let run = drive_async(
            &mut f,
            |_level| async move { Event::Accepted(10) },
            |_| {},
            &notify,
        );
        tokio::select! {
            () = run => panic!("drive_async returned on its own"),
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        assert_eq!(f.status().phase, Phase::Running);
        assert_eq!(f.status().funds_total, 10);
    }

    /// `notify.notify_one()` wakes a parked `Action::Stop` and calls `Fsm::next()`,
    /// exactly `Agent::next`'s own path -- a restartable cutoff resumes paying.
    #[tokio::test]
    async fn drive_async_resumes_a_stopped_wave_on_notify() {
        let t0 = Instant::now();
        let mut f = Paced::new(t0, LEVEL, Duration::ZERO, 1)
            .with_amount_cutoff(crate::consumer::paced::AmountCutoff::Relative(10));
        let notify = Notify::new();
        {
            let run = drive_async(
                &mut f,
                |_level| async move { Event::Accepted(10) },
                |_| {},
                &notify,
            );
            tokio::select! {
                () = run => panic!("drive_async returned on its own"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
        assert_eq!(f.status().funds_total, 10);
        notify.notify_one();
        {
            let run = drive_async(
                &mut f,
                |_level| async move { Event::Accepted(10) },
                |_| {},
                &notify,
            );
            tokio::select! {
                () = run => panic!("drive_async returned on its own"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
        assert_eq!(f.status().funds_total, 20);
    }
}
