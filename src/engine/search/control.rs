use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NO_DEADLINE: u64 = 0;

/// A clonable handle used to stop or retime a running search.
#[derive(Clone, Debug)]
pub struct SearchControl {
    shared: Arc<SharedControl>,
}

#[derive(Debug)]
struct SharedControl {
    epoch: Instant,
    stopped: AtomicBool,
    deadline_writer: Mutex<()>,
    deadline_version: AtomicU64,
    soft_deadline_nanos: AtomicU64,
    hard_deadline_nanos: AtomicU64,
}

impl SearchControl {
    /// Creates a control handle with no deadline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                epoch: Instant::now(),
                stopped: AtomicBool::new(false),
                deadline_writer: Mutex::new(()),
                deadline_version: AtomicU64::new(0),
                soft_deadline_nanos: AtomicU64::new(NO_DEADLINE),
                hard_deadline_nanos: AtomicU64::new(NO_DEADLINE),
            }),
        }
    }

    /// Requests that the search stop at its next cancellation check.
    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
    }

    /// Returns whether an explicit stop has been requested.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Relaxed)
    }

    /// Replaces both time limits with a duration measured from now.
    pub fn set_deadline_from_now(&self, duration: Duration) {
        self.set_time_budget_from_now(duration, duration);
    }

    pub(crate) fn set_time_budget_from_now(&self, soft: Duration, hard: Duration) {
        let now = self.shared.epoch.elapsed().as_nanos();
        let deadline = |duration: Duration| {
            now.saturating_add(duration.as_nanos())
                .min(u128::from(u64::MAX)) as u64
        };
        let soft_deadline = deadline(soft).max(1);
        let hard_deadline = deadline(hard.max(soft)).max(soft_deadline);
        self.write_deadlines(soft_deadline, hard_deadline);
    }

    /// Removes the current time limits without clearing an explicit stop.
    pub fn clear_deadline(&self) {
        self.write_deadlines(NO_DEADLINE, NO_DEADLINE);
    }

    pub(super) fn soft_deadline_reached(&self) -> bool {
        self.deadline_reached(self.deadline_snapshot().0)
    }

    pub(super) fn hard_deadline_reached(&self) -> bool {
        self.deadline_reached(self.deadline_snapshot().1)
    }

    pub(crate) fn has_time_budget(&self) -> bool {
        let (soft, hard) = self.deadline_snapshot();
        soft != NO_DEADLINE && hard != NO_DEADLINE
    }

    fn write_deadlines(&self, soft: u64, hard: u64) {
        let _guard = self
            .shared
            .deadline_writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.shared.deadline_version.fetch_add(1, Ordering::AcqRel);
        self.shared
            .soft_deadline_nanos
            .store(soft, Ordering::Relaxed);
        self.shared
            .hard_deadline_nanos
            .store(hard, Ordering::Relaxed);
        self.shared.deadline_version.fetch_add(1, Ordering::Release);
    }

    fn deadline_snapshot(&self) -> (u64, u64) {
        loop {
            let before = self.shared.deadline_version.load(Ordering::Acquire);
            if before % 2 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let soft = self.shared.soft_deadline_nanos.load(Ordering::Relaxed);
            let hard = self.shared.hard_deadline_nanos.load(Ordering::Relaxed);
            let after = self.shared.deadline_version.load(Ordering::Acquire);
            if before == after {
                return (soft, hard);
            }
        }
    }

    fn deadline_reached(&self, deadline: u64) -> bool {
        deadline != NO_DEADLINE && self.shared.epoch.elapsed().as_nanos() >= u128::from(deadline)
    }
}

impl Default for SearchControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SearchControl;

    #[test]
    fn stop_is_shared_between_clones() {
        let control = SearchControl::new();
        let clone = control.clone();

        clone.stop();

        assert!(control.is_stopped());
    }

    #[test]
    fn soft_and_hard_deadlines_can_be_set_and_cleared() {
        let control = SearchControl::new();
        assert!(!control.has_time_budget());

        control.set_time_budget_from_now(Duration::ZERO, Duration::from_secs(1));
        assert!(control.has_time_budget());
        assert!(control.soft_deadline_reached());
        assert!(!control.hard_deadline_reached());

        control.set_deadline_from_now(Duration::ZERO);
        assert!(control.hard_deadline_reached());

        control.clear_deadline();
        assert!(!control.has_time_budget());
        assert!(!control.soft_deadline_reached());
        assert!(!control.hard_deadline_reached());
    }
    #[test]
    fn concurrent_deadline_writers_publish_complete_pairs() {
        let control = SearchControl::new();
        let setter = control.clone();
        let clearer = control.clone();
        let setter = std::thread::spawn(move || {
            for _ in 0..2_000 {
                setter.set_time_budget_from_now(Duration::ZERO, Duration::from_secs(1));
            }
        });
        let clearer = std::thread::spawn(move || {
            for _ in 0..2_000 {
                clearer.clear_deadline();
                clearer.set_deadline_from_now(Duration::ZERO);
            }
        });

        for _ in 0..10_000 {
            let (soft, hard) = control.deadline_snapshot();
            assert!((soft == 0 && hard == 0) || (soft != 0 && hard >= soft));
        }
        setter.join().unwrap();
        clearer.join().unwrap();
    }
}
