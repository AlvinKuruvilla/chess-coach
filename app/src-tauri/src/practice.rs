//! The "find the move" trainer: which of your past mistakes to show next, and
//! when a position comes back.
//!
//! A drill is the position before one of your mistakes. You get one attempt,
//! a hint and one retry, then the answer. How it went decides when you see
//! the position again, on a Leitner schedule:
//!
//! ```text
//!   box      0        1       2       3       4        5
//!   wait   10 min   1 day   3 days  7 days  21 days  60 days
//!
//!   solved on the first try   -> next box (5 stays 5)
//!   solved on the retry       -> same box, back tomorrow
//!   missed                    -> box 0, back in ten minutes
//! ```
//!
//! The intervals are the usual expanding spacing, picked by hand rather than
//! fitted: the question of how often a beginner should see a chess position
//! again has no measured answer that was found (2026-11-01). Ten minutes for a
//! miss means it comes back in the same sitting, after other positions, not
//! straight away, when the answer would still be on screen in your head.

use serde::{Deserialize, Serialize};

/// Expected score lost below which a move counts as finding it: under
/// Lichess's inaccuracy line (5 points; see `docs/win-curve.md`). A move
/// that loses less than that is as good as the engine's for a player at
/// this level, even if it is not the same move.
pub const SOLVED_POINTS: f64 = 5.0;

const MINUTE: i64 = 60;
const DAY: i64 = 24 * 60 * MINUTE;

/// Seconds to wait before showing a position in this box again.
const WAIT: [i64; 6] = [10 * MINUTE, DAY, 3 * DAY, 7 * DAY, 21 * DAY, 60 * DAY];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    FirstTry,
    Retry,
    Missed,
}

/// Where a drill stands: its box and when it is next due, Unix seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub r#box: u32,
    pub due_at: i64,
}

/// The schedule after answering a drill in `current` box at time `now`.
pub fn next(current: u32, grade: Grade, now: i64) -> Schedule {
    let last = u32::try_from(WAIT.len() - 1).expect("six boxes fit in u32");
    let wait_for = |b: u32| WAIT[usize::try_from(b).expect("a box index fits in usize")];
    let (r#box, wait) = match grade {
        Grade::FirstTry => {
            let r#box = (current + 1).min(last);
            (r#box, wait_for(r#box))
        }
        Grade::Retry => (current.min(last), DAY),
        Grade::Missed => (0, wait_for(0)),
    };
    Schedule {
        r#box,
        due_at: now + wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_try_climbs_one_box_and_waits_longer() {
        assert_eq!(
            next(0, Grade::FirstTry, 0),
            Schedule {
                r#box: 1,
                due_at: DAY
            }
        );
        assert_eq!(
            next(3, Grade::FirstTry, 0),
            Schedule {
                r#box: 4,
                due_at: 21 * DAY
            }
        );
        // The top box is the ceiling.
        assert_eq!(
            next(5, Grade::FirstTry, 0),
            Schedule {
                r#box: 5,
                due_at: 60 * DAY
            }
        );
    }

    #[test]
    fn a_retry_holds_the_box_and_a_miss_starts_over() {
        assert_eq!(
            next(3, Grade::Retry, 100),
            Schedule {
                r#box: 3,
                due_at: 100 + DAY
            }
        );
        assert_eq!(
            next(4, Grade::Missed, 100),
            Schedule {
                r#box: 0,
                due_at: 100 + 10 * MINUTE
            }
        );
    }
}
