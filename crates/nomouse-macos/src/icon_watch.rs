//! Whether the menu bar item really came up, and what to do when it did not.
//!
//! macOS can accept a status item and still never draw it: the menu bar refuses the item's
//! scene (it did so for a process that replaced itself with `exec`), AppKit asks again once a
//! second, and the icon never shows. [`IconWatch`] looks a little after the item was made and
//! now and then after that; an item that is not on screen is made again a few times, further
//! apart each time, and then given up, so the failure is reported once and nothing retries
//! every second.

use std::time::{Duration, Instant};

/// How long the menu bar gets to show a new item before the first look.
pub const SETTLE: Duration = Duration::from_secs(3);

/// How often an item that is on screen is looked at again.
pub const RECHECK: Duration = Duration::from_secs(10);

/// How long to wait after each new attempt: one entry per retry.
pub const BACKOFF: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(20),
    Duration::from_secs(60),
];

/// What the owner of the item does now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing: not time to look yet, or the item is still on screen.
    Wait,
    /// The item is on screen after `retries` new attempts (0: the first one worked).
    Appeared {
        /// How many times the item was made again before it showed.
        retries: usize,
    },
    /// The item is not on screen: make it again. `first` is the first failure since it
    /// was last on screen, the one to report.
    Retry {
        /// Whether this is the first failure since the item was last on screen.
        first: bool,
    },
    /// The item stayed off screen through every retry: take it out and tell the user once.
    /// The watch never looks again.
    GiveUp,
}

/// The schedule of looks at one menu bar item.
#[derive(Debug, Clone)]
pub struct IconWatch {
    next: Instant,
    /// New attempts since the item was last on screen.
    retries: usize,
    /// Whether the item was on screen at the last look.
    shown: bool,
    gave_up: bool,
}

impl IconWatch {
    /// A watch over an item made at `now`.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            next: now + SETTLE,
            retries: 0,
            shown: false,
            gave_up: false,
        }
    }

    /// Looks at the item when it is time to (`on_screen` is asked only then) and says what
    /// to do.
    pub fn check(&mut self, now: Instant, on_screen: impl FnOnce() -> bool) -> Verdict {
        if self.gave_up || now < self.next {
            return Verdict::Wait;
        }
        if on_screen() {
            self.next = now + RECHECK;
            let appeared = !self.shown;
            self.shown = true;
            let retries = std::mem::take(&mut self.retries);
            return if appeared {
                Verdict::Appeared { retries }
            } else {
                Verdict::Wait
            };
        }
        self.shown = false;
        if let Some(wait) = BACKOFF.get(self.retries) {
            self.retries += 1;
            self.next = now + *wait;
            Verdict::Retry {
                first: self.retries == 1,
            }
        } else {
            self.gave_up = true;
            Verdict::GiveUp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the watch once a second for `seconds`, with the item on screen as `on_screen`
    /// says for each second, and returns every verdict that is not `Wait`, with its second.
    fn run(seconds: u64, on_screen: impl Fn(u64) -> bool) -> Vec<(u64, Verdict)> {
        let start = Instant::now();
        let mut watch = IconWatch::new(start);
        (0..=seconds)
            .filter_map(|second| {
                let verdict =
                    watch.check(start + Duration::from_secs(second), || on_screen(second));
                (verdict != Verdict::Wait).then_some((second, verdict))
            })
            .collect()
    }

    #[test]
    fn an_item_on_screen_is_reported_once_after_it_settles() {
        assert_eq!(
            run(600, |_| true),
            vec![(3, Verdict::Appeared { retries: 0 })]
        );
    }

    #[test]
    fn nothing_is_asked_before_the_item_settles() {
        let start = Instant::now();
        let mut watch = IconWatch::new(start);
        let verdict = watch.check(start + Duration::from_secs(2), || {
            panic!("looked before the menu bar had time to show the item")
        });
        assert_eq!(verdict, Verdict::Wait);
    }

    #[test]
    fn a_refused_item_is_retried_further_apart_then_given_up_once() {
        assert_eq!(
            run(3600, |_| false),
            vec![
                (3, Verdict::Retry { first: true }),
                (8, Verdict::Retry { first: false }),
                (28, Verdict::Retry { first: false }),
                (88, Verdict::GiveUp),
            ]
        );
    }

    #[test]
    fn an_item_that_shows_after_a_retry_says_how_many_it_took() {
        assert_eq!(
            run(600, |second| second >= 8),
            vec![
                (3, Verdict::Retry { first: true }),
                (8, Verdict::Appeared { retries: 1 }),
            ]
        );
    }

    #[test]
    fn an_item_lost_later_is_retried_and_reported_again() {
        let verdicts = run(600, |second| !(20..30).contains(&second));
        assert_eq!(
            verdicts,
            vec![
                (3, Verdict::Appeared { retries: 0 }),
                (23, Verdict::Retry { first: true }),
                (28, Verdict::Retry { first: false }),
                (48, Verdict::Appeared { retries: 2 }),
            ]
        );
    }
}
