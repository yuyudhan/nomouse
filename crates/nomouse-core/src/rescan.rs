//! When hint mode scans again while the user has typed nothing: a page that is still loading
//! (a browser right after a navigation, or one whose accessibility tree was just switched on)
//! adds its targets after the first scan, and the user may be waiting for them.

use std::time::{Duration, Instant};

/// When the re-scans run, from the hotkey (in continuous mode, from the scan after each
/// action): one right after the first scan, then every `hints.early_rescan_ms` until
/// `hints.late_rescan_ms`, and at `hints.late_rescan_ms` itself (1, 2 and 3 s by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescanSchedule {
    /// The time between stages; `None` when stages come only at `end` (or not at all).
    every: Option<Duration>,
    /// The last stage; zero when there is none after the immediate re-scan.
    end: Duration,
}

impl RescanSchedule {
    /// The immediate re-scan, then stages every `early_ms` milliseconds up to `late_ms` and
    /// one at `late_ms`. A stage of 0 is left out: `early_ms` 0 leaves the immediate re-scan
    /// and `late_ms`; `late_ms` 0, or not later than `early_ms`, leaves one stage at
    /// `early_ms`.
    pub fn new(early_ms: u32, late_ms: u32) -> Self {
        let early = Duration::from_millis(u64::from(early_ms));
        let late = Duration::from_millis(u64::from(late_ms));
        if early.is_zero() {
            Self {
                every: None,
                end: late,
            }
        } else if late > early {
            Self {
                every: Some(early),
                end: late,
            }
        } else {
            Self {
                every: None,
                end: early,
            }
        }
    }

    /// The time of re-scan number `index` (0 is the immediate one); `None` past the last.
    fn at(&self, index: u32) -> Option<Duration> {
        // Interval stages are the multiples of `every` before `end`; `end` follows them.
        let last_interval = self.every.map_or(0, |every| {
            let below_end = self.end.as_nanos().saturating_sub(1) / every.as_nanos().max(1);
            u32::try_from(below_end).unwrap_or(u32::MAX)
        });
        if index <= last_interval {
            return Some(self.every.map_or(Duration::ZERO, |every| every * index));
        }
        (Some(index) == last_interval.checked_add(1) && !self.end.is_zero()).then_some(self.end)
    }

    /// The re-scan times in order, the immediate one first.
    pub fn times(&self) -> Vec<Duration> {
        (0..).map_while(|index| self.at(index)).collect()
    }
}

impl Default for RescanSchedule {
    fn default() -> Self {
        Self::new(1000, 3000)
    }
}

/// The re-scans still pending for one set of labels, per a [`RescanSchedule`]. Typing does not
/// stop them: only picking a target or closing the labels ends the activation, and with it
/// its re-scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rescans {
    start: Instant,
    schedule: RescanSchedule,
    /// Number of the next re-scan in the schedule.
    next: u32,
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
        while self.schedule.at(self.next).is_some_and(|at| at <= elapsed) {
            self.next += 1;
        }
        self.next > before
    }

    /// How long from `now` until the next re-scan is due (zero when one is due); `None` once
    /// none is pending.
    pub fn until_next(&self, now: Instant) -> Option<Duration> {
        self.schedule
            .at(self.next)
            .map(|at| (self.start + at).saturating_duration_since(now))
    }

    /// Whether a re-scan is still pending.
    pub fn is_pending(&self) -> bool {
        self.schedule.at(self.next).is_some()
    }
}

/// The least time between two scans a change of the tree starts: a loading page reports many
/// changes in a row, and each scan is a walk of the window.
pub const CHANGE_GAP: Duration = Duration::from_millis(200);

/// Scans asked for by the platform reporting that the tree changed
/// ([`crate::InputEvent::TreeChanged`]: a page loaded, elements went away): one runs as soon
/// as the last scan is at least [`CHANGE_GAP`] old, so a page that arrives between two
/// scheduled re-scans gets its labels at once rather than at the next stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changes {
    /// When the last scan started.
    last_scan: Instant,
    /// When the first change not yet covered by a scan was reported.
    changed: Option<Instant>,
}

impl Changes {
    /// No change pending; the last scan started at `scanned`.
    pub const fn new(scanned: Instant) -> Self {
        Self {
            last_scan: scanned,
            changed: None,
        }
    }

    /// The tree changed at `now`.
    pub fn changed(&mut self, now: Instant) {
        self.changed.get_or_insert(now);
    }

    /// A scan started at `at`: it covers every change reported before it.
    pub fn scanned(&mut self, at: Instant) {
        self.last_scan = at;
        if self.changed.is_some_and(|changed| changed <= at) {
            self.changed = None;
        }
    }

    /// Whether a scan for a change is due at `now`.
    pub fn is_due(&self, now: Instant) -> bool {
        self.until_due(now).is_some_and(|wait| wait.is_zero())
    }

    /// How long from `now` until a scan for a change is due (zero when one is due); `None`
    /// when no change is pending.
    pub fn until_due(&self, now: Instant) -> Option<Duration> {
        self.changed
            .map(|_| (self.last_scan + CHANGE_GAP).saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_change_scans_at_once_unless_the_last_scan_is_recent() {
        let start = Instant::now();
        let mut changes = Changes::new(start);
        assert!(!changes.is_due(start + 5000 * MS), "no change, no scan");
        assert_eq!(changes.until_due(start + 5000 * MS), None);
        // A change 400 ms after the last scan: due at once.
        changes.changed(start + 400 * MS);
        assert!(changes.is_due(start + 400 * MS));
        changes.scanned(start + 400 * MS);
        assert!(!changes.is_due(start + 400 * MS));
        // Changes 50 ms after a scan wait until it is 200 ms old, then one scan covers them.
        changes.changed(start + 450 * MS);
        changes.changed(start + 500 * MS);
        assert_eq!(changes.until_due(start + 450 * MS), Some(150 * MS));
        assert!(!changes.is_due(start + 599 * MS));
        assert!(changes.is_due(start + 600 * MS));
        changes.scanned(start + 600 * MS);
        assert_eq!(changes.until_due(start + 600 * MS), None);
    }

    #[test]
    fn a_scan_that_started_before_a_change_does_not_cover_it() {
        let start = Instant::now();
        let mut changes = Changes::new(start);
        changes.changed(start + 1100 * MS);
        // The 1 s stage started before the change: the change still needs its own scan.
        changes.scanned(start + 1000 * MS);
        assert!(changes.is_due(start + 1200 * MS));
    }

    #[test]
    fn rescans_come_at_once_then_every_second_until_three_seconds() {
        let start = Instant::now();
        let mut rescans = Rescans::new(start, RescanSchedule::default());
        assert!(rescans.take_due(start + 20 * MS));
        assert!(!rescans.take_due(start + 20 * MS));
        assert_eq!(rescans.until_next(start + 20 * MS), Some(980 * MS));
        assert!(!rescans.take_due(start + 999 * MS));
        assert!(rescans.take_due(start + 1000 * MS));
        assert_eq!(rescans.until_next(start + 1500 * MS), Some(500 * MS));
        assert!(!rescans.take_due(start + 1999 * MS));
        assert!(rescans.take_due(start + 2000 * MS));
        assert!(!rescans.take_due(start + 2999 * MS));
        assert!(rescans.take_due(start + 3000 * MS));
        assert!(!rescans.is_pending());
        assert_eq!(rescans.until_next(start + 3000 * MS), None);
        assert!(!rescans.take_due(start + 9000 * MS));
    }

    #[test]
    fn the_stages_follow_the_settings() {
        // Every `early` until `late`, and `late` itself.
        assert_eq!(
            RescanSchedule::new(1000, 3000).times(),
            [Duration::ZERO, 1000 * MS, 2000 * MS, 3000 * MS]
        );
        assert_eq!(
            RescanSchedule::new(500, 1800).times(),
            [Duration::ZERO, 500 * MS, 1000 * MS, 1500 * MS, 1800 * MS]
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
        assert_eq!(rescans.until_next(start + 1200 * MS), Some(800 * MS));
        assert!(rescans.take_due(start + 5000 * MS));
        assert!(!rescans.is_pending());
    }
}
