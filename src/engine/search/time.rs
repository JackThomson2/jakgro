use std::time::Duration;

use cozy_chess::Color;

use super::SearchLimits;

const DEFAULT_MOVES_TO_GO: u32 = 30;
const MIN_RESERVE: Duration = Duration::from_millis(5);
const MAX_RESERVE: Duration = Duration::from_millis(100);
const HARD_TIME_MULTIPLIER: u32 = 3;
pub(crate) const MIN_MOVE_OVERHEAD_MS: u64 = 0;
pub(crate) const DEFAULT_MOVE_OVERHEAD_MS: u64 = 10;
pub(crate) const MAX_MOVE_OVERHEAD_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeBudget {
    soft: Duration,
    hard: Duration,
}

impl TimeBudget {
    fn fixed(duration: Duration) -> Self {
        Self {
            soft: duration,
            hard: duration,
        }
    }

    fn managed(soft: Duration, usable: Duration) -> Self {
        Self {
            soft,
            hard: soft
                .saturating_mul(HARD_TIME_MULTIPLIER)
                .min(usable)
                .max(soft),
        }
    }

    pub(crate) fn soft(self) -> Duration {
        self.soft
    }

    pub(crate) fn hard(self) -> Duration {
        self.hard
    }
}

pub(super) fn allocate_time(
    side_to_move: Color,
    limits: &SearchLimits,
    move_overhead: Duration,
) -> Option<TimeBudget> {
    if limits.infinite || limits.ponder {
        return None;
    }
    if let Some(move_time) = limits.move_time {
        return Some(TimeBudget::fixed(move_time));
    }

    let (remaining, increment) = match side_to_move {
        Color::White => (limits.white_time?, limits.white_increment),
        Color::Black => (limits.black_time?, limits.black_increment),
    };
    let safe_remaining = remaining.saturating_sub(move_overhead);
    let reserve = (safe_remaining / 20)
        .clamp(MIN_RESERVE, MAX_RESERVE)
        .min(safe_remaining);
    let usable = safe_remaining.saturating_sub(reserve);
    if usable.is_zero() {
        return Some(TimeBudget::fixed(Duration::ZERO));
    }

    let moves_to_go = limits.moves_to_go.unwrap_or(DEFAULT_MOVES_TO_GO).max(1);
    let increment_share = increment.unwrap_or_default().saturating_mul(3) / 4;
    let soft = (usable / moves_to_go)
        .saturating_add(increment_share)
        .min(usable)
        .max(Duration::from_millis(1).min(usable));
    Some(TimeBudget::managed(soft, usable))
}
pub(super) fn allocate_time_after_ponder(
    side_to_move: Color,
    limits: &SearchLimits,
    move_overhead: Duration,
) -> Option<TimeBudget> {
    let mut active_limits = limits.clone();
    active_limits.ponder = false;
    allocate_time(side_to_move, &active_limits, move_overhead)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cozy_chess::Color;

    use super::{TimeBudget, allocate_time, allocate_time_after_ponder};
    use crate::engine::SearchLimits;

    const OVERHEAD: Duration = Duration::from_millis(10);

    #[test]
    fn move_time_has_precedence_and_fixed_limits() {
        let limits = SearchLimits {
            move_time: Some(Duration::from_millis(250)),
            white_time: Some(Duration::from_secs(30)),
            ..SearchLimits::default()
        };

        let expected = Some(TimeBudget::fixed(Duration::from_millis(250)));
        assert_eq!(allocate_time(Color::White, &limits, OVERHEAD), expected);
        assert_eq!(
            allocate_time(Color::White, &limits, Duration::ZERO),
            expected
        );
    }

    #[test]
    fn clocks_have_soft_time_and_bounded_hard_time() {
        let limits = SearchLimits {
            white_time: Some(Duration::from_secs(30)),
            white_increment: Some(Duration::from_secs(1)),
            moves_to_go: Some(30),
            ..SearchLimits::default()
        };

        let budget = allocate_time(Color::White, &limits, OVERHEAD).unwrap();
        let without_overhead = allocate_time(Color::White, &limits, Duration::ZERO).unwrap();

        assert!(budget.soft() > Duration::from_secs(1));
        assert!(budget.soft() < without_overhead.soft());
        assert!(budget.hard() < without_overhead.hard());
        assert!(budget.soft() < Duration::from_secs(2));
        assert_eq!(budget.hard(), budget.soft().saturating_mul(3));
        assert!(budget.hard() <= Duration::from_millis(29_890));
    }

    #[test]
    fn allocation_uses_the_side_to_move_clock() {
        let limits = SearchLimits {
            white_time: Some(Duration::from_secs(30)),
            black_time: Some(Duration::from_secs(3)),
            moves_to_go: Some(10),
            ..SearchLimits::default()
        };

        assert!(
            allocate_time(Color::Black, &limits, OVERHEAD)
                .unwrap()
                .soft()
                < allocate_time(Color::White, &limits, OVERHEAD)
                    .unwrap()
                    .soft()
        );
    }

    #[test]
    fn overhead_larger_than_the_clock_saturates_both_limits() {
        let limits = SearchLimits {
            white_time: Some(Duration::from_millis(5)),
            ..SearchLimits::default()
        };

        assert_eq!(
            allocate_time(Color::White, &limits, OVERHEAD),
            Some(TimeBudget::fixed(Duration::ZERO))
        );
    }
    #[test]
    fn extreme_increments_saturate_without_exceeding_usable_time() {
        let limits = SearchLimits {
            white_time: Some(Duration::MAX),
            white_increment: Some(Duration::MAX),
            ..SearchLimits::default()
        };

        let budget = allocate_time(Color::White, &limits, OVERHEAD).unwrap();

        assert!(budget.hard() >= budget.soft());
        assert!(budget.hard() < Duration::MAX);
    }

    #[test]
    fn infinite_and_ponder_searches_have_no_initial_deadline() {
        let infinite = SearchLimits {
            infinite: true,
            move_time: Some(Duration::from_secs(1)),
            ..SearchLimits::default()
        };
        let ponder = SearchLimits {
            ponder: true,
            move_time: Some(Duration::from_secs(1)),
            ..SearchLimits::default()
        };

        assert_eq!(allocate_time(Color::White, &infinite, OVERHEAD), None);
        assert_eq!(allocate_time(Color::White, &ponder, OVERHEAD), None);
    }

    #[test]
    fn ponderhit_activates_the_fixed_movetime_budget() {
        let limits = SearchLimits {
            ponder: true,
            move_time: Some(Duration::from_millis(250)),
            ..SearchLimits::default()
        };

        assert_eq!(allocate_time(Color::White, &limits, OVERHEAD), None);
        assert_eq!(
            allocate_time_after_ponder(Color::White, &limits, OVERHEAD),
            Some(TimeBudget::fixed(Duration::from_millis(250)))
        );
    }
}
