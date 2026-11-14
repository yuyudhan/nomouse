//! When hint mode scans again while the user has typed nothing: a page that is still loading
//! (a browser right after a navigation, or one whose accessibility tree was just switched on)
//! adds its targets after the first scan, and the user may be waiting for them.

use std::time::{Duration, Instant};

/// When the re-scans run, from the hotkey (in continuous mode, from the scan after each
/// action): one right after the first scan, then the early and late stages
/// (`hints.early_rescan_ms`, `hints.late_rescan_ms`; 1 s and 3 s by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescanSchedule {
    /// The re-scan times in increasing order; only the first `len` are used.
    at: [Duration; 3],
    len: usize,
}

impl RescanSchedule {
    /// The immediate re-scan plus a stage at `early_ms` and one at `late_ms` milliseconds; a
    /// stage of 0 is left out, and so is one not later than the stage before it.
    pub fn new(early_ms: u32, late_ms: u32) -> Self {
        let mut schedule = Self {
            at: [Duration::ZERO; 3],
            len: 1,
        };
        for ms in [early_ms, late_ms] {
            let at = Duration::from_millis(u64::from(ms));
            if at > schedule.at[schedule.len - 1] {
                schedule.at[schedule.len] = at;
                schedule.len += 1;
            }
        }
        schedule
    }

    /// The re-scan times in order, the immediate one first.
    pub fn times(&self) -> &[Duration] {
        &self.at[..self.len]
    }
}

impl Default for RescanSchedule {
    fn default() -> Self {
        Self::new(1000, 3000)
    }
}

/// The re-scans still pending for one set of labels, per a [`RescanSchedule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rescans {
    start: Instant,
    schedule: RescanSchedule,
    /// Index into the schedule's times of the next re-scan.
    next: usize,
}

impl Rescans {
    /// Every re-scan of `schedule` pending, counted from `start`.
    pub const fn new(start: Instant, schedule: RescanSchedule) -> Self {
        Self {
            start,
            schedule,
            next: 0,
        }
    }

    /// Whether a re-scan is due at `now`. A due re-scan is consumed, together with every
    /// later one that is also due already (a slow first scan runs one re-scan, not a burst).
    pub fn take_due(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.start);
        let before = self.next;
        while self
            .schedule
            .times()
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
        self.schedule
            .times()
            .get(self.next)
            .map(|at| (self.start + *at).saturating_duration_since(now))
    }

    /// Drops every pending re-scan: the user typed a key.
    pub const fn cancel(&mut self) {
        self.next = self.schedule.len;
    }

    /// Whether a re-scan is still pending.
    pub const fn is_pending(&self) -> bool {
        self.next < self.schedule.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn rescans_come_at_once_then_at_one_and_three_seconds() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start, RescanSchedule::default());
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
    fn the_stages_follow_the_settings() {
        assert_eq!(
            RescanSchedule::new(500, 5000).times(),
            [Duration::ZERO, 500 * MS, 5000 * MS]
        );
        // 0 turns a stage off; with both off only the immediate re-scan is left.
        assert_eq!(
            RescanSchedule::new(0, 2000).times(),
            [Duration::ZERO, 2000 * MS]
        );
        assert_eq!(
            RescanSchedule::new(1000, 0).times(),
            [Duration::ZERO, 1000 * MS]
        );
        assert_eq!(RescanSchedule::new(0, 0).times(), [Duration::ZERO]);
        // A late stage not after the early one adds nothing (validation rejects it anyway).
        assert_eq!(
            RescanSchedule::new(3000, 3000).times(),
            [Duration::ZERO, 3000 * MS]
        );
        let start = Instant::now();
        let mut once = Rescans::new(start, RescanSchedule::new(0, 0));
        assert!(once.take_due(start));
        assert!(!once.is_pending());
    }

    #[test]
    fn overdue_rescans_run_once() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start, RescanSchedule::default());
        // The first scan took 1.2 s: the immediate and the 1 s re-scans are one.
        assert!(rescans.take_due(start + 1200 * MS));
        assert_eq!(rescans.until_next(start + 1200 * MS), Some(1800 * MS));
        assert!(rescans.take_due(start + 5000 * MS));
        assert!(!rescans.is_pending());
    }

    #[test]
    fn cancel_drops_every_pending_rescan() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start, RescanSchedule::default());
        rescans.cancel();
        assert!(!rescans.is_pending());
        assert!(!rescans.take_due(start + 4000 * MS));
        assert_eq!(rescans.until_next(start), None);
    }
}
