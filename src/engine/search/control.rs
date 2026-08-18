use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    deadline_nanos: AtomicU64,
}

impl SearchControl {
    /// Creates a control handle with no deadline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                epoch: Instant::now(),
                stopped: AtomicBool::new(false),
                deadline_nanos: AtomicU64::new(NO_DEADLINE),
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

    /// Replaces the deadline with a duration measured from now.
    pub fn set_deadline_from_now(&self, duration: Duration) {
        let deadline = self
            .shared
            .epoch
            .elapsed()
            .as_nanos()
            .saturating_add(duration.as_nanos())
            .min(u128::from(u64::MAX)) as u64;
        self.shared
            .deadline_nanos
            .store(deadline.max(1), Ordering::Relaxed);
    }

    /// Removes the current deadline without clearing an explicit stop.
    pub fn clear_deadline(&self) {
        self.shared
            .deadline_nanos
            .store(NO_DEADLINE, Ordering::Relaxed);
    }

    pub(super) fn deadline_reached(&self) -> bool {
        let deadline = self.shared.deadline_nanos.load(Ordering::Relaxed);
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
    fn deadlines_can_be_set_and_cleared() {
        let control = SearchControl::new();

        control.set_deadline_from_now(Duration::ZERO);
        assert!(control.deadline_reached());

        control.clear_deadline();
        assert!(!control.deadline_reached());
    }
}
