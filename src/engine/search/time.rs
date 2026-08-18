use std::time::Duration;

use cozy_chess::Color;

use super::SearchLimits;

const DEFAULT_MOVES_TO_GO: u32 = 30;
const MIN_RESERVE: Duration = Duration::from_millis(5);
const MAX_RESERVE: Duration = Duration::from_millis(100);

pub(super) fn allocate_time(side_to_move: Color, limits: &SearchLimits) -> Option<Duration> {
    if limits.infinite || limits.ponder {
        return None;
    }
    if let Some(move_time) = limits.move_time {
        return Some(move_time);
    }

    let (remaining, increment) = match side_to_move {
        Color::White => (limits.white_time?, limits.white_increment),
        Color::Black => (limits.black_time?, limits.black_increment),
    };
    let reserve = (remaining / 20)
        .clamp(MIN_RESERVE, MAX_RESERVE)
        .min(remaining);
    let usable = remaining.saturating_sub(reserve);
    if usable.is_zero() {
        return Some(Duration::ZERO);
    }

    let moves_to_go = limits.moves_to_go.unwrap_or(DEFAULT_MOVES_TO_GO).max(1);
    let increment_share = increment.unwrap_or_default() * 3 / 4;
    let allocation = (usable / moves_to_go)
        .saturating_add(increment_share)
        .min(usable);
    Some(allocation.max(Duration::from_millis(1).min(usable)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cozy_chess::Color;

    use super::allocate_time;
    use crate::engine::SearchLimits;

    #[test]
    fn move_time_has_precedence() {
        let limits = SearchLimits {
            move_time: Some(Duration::from_millis(250)),
            white_time: Some(Duration::from_secs(30)),
            ..SearchLimits::default()
        };

        assert_eq!(
            allocate_time(Color::White, &limits),
            Some(Duration::from_millis(250))
        );
    }

    #[test]
    fn clocks_include_part_of_the_increment_and_keep_a_reserve() {
        let limits = SearchLimits {
            white_time: Some(Duration::from_secs(30)),
            white_increment: Some(Duration::from_secs(1)),
            moves_to_go: Some(30),
            ..SearchLimits::default()
        };

        let allocation = allocate_time(Color::White, &limits).unwrap();

        assert!(allocation > Duration::from_secs(1));
        assert!(allocation < Duration::from_secs(2));
        assert!(allocation < limits.white_time.unwrap());
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
            allocate_time(Color::Black, &limits).unwrap()
                < allocate_time(Color::White, &limits).unwrap()
        );
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

        assert_eq!(allocate_time(Color::White, &infinite), None);
        assert_eq!(allocate_time(Color::White, &ponder), None);
    }
}
