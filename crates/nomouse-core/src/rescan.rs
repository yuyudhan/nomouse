//! When hint mode scans again while the user has typed nothing: a page that is still loading
//! (a browser right after a navigation) adds its targets after the first scan, and the user
//! may be waiting for them.

use std::time::{Duration, Instant};

/// When the re-scans run, from the hotkey (in continuous mode, from the scan after each
/// action): one right after the first scan, one at 1 s and the last at 3 s.
pub const RESCAN_SCHEDULE: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(1000),
    Duration::from_millis(3000),
];

/// The re-scans still pending for one set of labels, per [`RESCAN_SCHEDULE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rescans {
    start: Instant,
    /// Index into [`RESCAN_SCHEDULE`] of the next re-scan.
    next: usize,
}

impl Rescans {
    /// Every re-scan pending, counted from `start`.
    pub const fn new(start: Instant) -> Self {
        Self { start, next: 0 }
    }

    /// Whether a re-scan is due at `now`. A due re-scan is consumed, together with every
    /// later one that is also due already (a slow first scan runs one re-scan, not a burst).
    pub fn take_due(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.start);
        let before = self.next;
        while RESCAN_SCHEDULE
            .get(self.next)
            .is_some_and(|at| *at <= elapsed)
        {
            self.next += 1;
        }
        self.next > before
    }

    /// How long from `now` until the next re-scan is due (zero when one is due); `None` once
    /// none is pending.
    pub fn until_next(&self, now: Instant) -> Option<Duration> {
        RESCAN_SCHEDULE
            .get(self.next)
            .map(|at| (self.start + *at).saturating_duration_since(now))
    }

    /// Drops every pending re-scan: the user typed a key.
    pub const fn cancel(&mut self) {
        self.next = RESCAN_SCHEDULE.len();
    }

    /// Whether a re-scan is still pending.
    pub const fn is_pending(&self) -> bool {
        self.next < RESCAN_SCHEDULE.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn rescans_come_at_once_then_at_one_and_three_seconds() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start);
        assert!(rescans.take_due(start + 20 * MS));
        assert!(!rescans.take_due(start + 20 * MS));
        assert_eq!(rescans.until_next(start + 20 * MS), Some(980 * MS));
        assert!(!rescans.take_due(start + 999 * MS));
        assert!(rescans.take_due(start + 1000 * MS));
        assert_eq!(rescans.until_next(start + 1500 * MS), Some(1500 * MS));
        assert!(!rescans.take_due(start + 2999 * MS));
        assert!(rescans.take_due(start + 3000 * MS));
        assert!(!rescans.is_pending());
        assert_eq!(rescans.until_next(start + 3000 * MS), None);
        assert!(!rescans.take_due(start + 9000 * MS));
    }

    #[test]
    fn overdue_rescans_run_once() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start);
        // The first scan took 1.2 s: the immediate and the 1 s re-scans are one.
        assert!(rescans.take_due(start + 1200 * MS));
        assert_eq!(rescans.until_next(start + 1200 * MS), Some(1800 * MS));
        assert!(rescans.take_due(start + 5000 * MS));
        assert!(!rescans.is_pending());
    }

    #[test]
    fn cancel_drops_every_pending_rescan() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start);
        rescans.cancel();
        assert!(!rescans.is_pending());
        assert!(!rescans.take_due(start + 4000 * MS));
        assert_eq!(rescans.until_next(start), None);
    }
}
