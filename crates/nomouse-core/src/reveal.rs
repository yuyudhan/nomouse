//! Waiting for an element scrolled into view to come to rest.
//!
//! Picking an element that was off-screen (`hints.browser = "exhaustive"`) asks its toolkit to
//! scroll it into view and then acts where it ended up. Browsers scroll smoothly and report the
//! element's final place while they are still moving, and they ignore a click meanwhile; an
//! accessibility tree also lags behind the scroll. Acting on the first reading that lies in
//! view therefore misses intermittently, so backends act only on a reading that several polls in
//! a row repeated.

/// Polls `read` until `steady` consecutive readings are equal and `in_view`, then returns that
/// reading. `wait` is called before every poll: it pauses and returns whether polling should
/// go on (`false` once the backend's deadline passed). `None` when the deadline passed first:
/// the element never came to rest in view (a reading that could not be taken, `None`, restarts
/// the count), and acting on it would click wherever it last happened to be.
///
/// `steady` is how often a reading must repeat its predecessor; `0` accepts the first reading
/// in view, which is the race this function exists to avoid.
pub fn settle<T: Copy + PartialEq>(
    mut read: impl FnMut() -> Option<T>,
    in_view: impl Fn(T) -> bool,
    steady: u32,
    mut wait: impl FnMut() -> bool,
) -> Option<T> {
    let mut last: Option<T> = None;
    let mut repeats = 0u32;
    while wait() {
        let reading = read();
        repeats = match (reading, last) {
            (Some(now), Some(before)) if now == before => repeats.saturating_add(1),
            _ => 0,
        };
        last = reading;
        if let Some(now) = reading
            && repeats >= steady
            && in_view(now)
        {
            return Some(now);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `settle` over a fixed series of readings, `polls` polls at most.
    fn run(
        readings: &[Option<i32>],
        steady: u32,
        polls: usize,
        in_view: fn(i32) -> bool,
    ) -> Option<i32> {
        let mut next = readings.iter().copied();
        let mut left = polls;
        settle(
            || next.next().flatten(),
            in_view,
            steady,
            || {
                let more = left > 0;
                left = left.saturating_sub(1);
                more
            },
        )
    }

    fn anywhere(_: i32) -> bool {
        true
    }

    #[test]
    fn waits_for_a_smooth_scroll_to_end() {
        // The element is "in view" from the first reading on, while the page is still moving:
        // 900, 700, 500, then at rest at 400.
        let scroll = [
            Some(900),
            Some(700),
            Some(500),
            Some(400),
            Some(400),
            Some(400),
            Some(400),
        ];
        assert_eq!(run(&scroll, 3, 10, anywhere), Some(400));
        // Accepting the first reading in view (steady = 0) acts on the page mid-scroll.
        assert_eq!(run(&scroll, 0, 10, anywhere), Some(900));
    }

    #[test]
    fn a_reading_that_is_not_in_view_never_counts() {
        // At rest below the viewport (a stale frame the toolkit has not updated yet).
        let stale = [Some(6000); 10];
        assert_eq!(run(&stale, 3, 10, |at| at < 1000), None);
        // The frame arrives late and is then steady.
        let late = [
            Some(6000),
            Some(6000),
            Some(6000),
            Some(400),
            Some(400),
            Some(400),
            Some(400),
        ];
        assert_eq!(run(&late, 3, 10, |at| at < 1000), Some(400));
    }

    #[test]
    fn a_failed_read_restarts_the_count() {
        let flaky = [
            Some(400),
            Some(400),
            None,
            Some(400),
            Some(400),
            Some(400),
            Some(400),
        ];
        assert_eq!(run(&flaky, 3, 3, anywhere), None);
        assert_eq!(run(&flaky, 3, 7, anywhere), Some(400));
    }

    #[test]
    fn gives_up_when_the_deadline_passes_first() {
        let moving: Vec<Option<i32>> = (0..50).map(Some).collect();
        assert_eq!(run(&moving, 3, 50, anywhere), None);
        assert_eq!(run(&[Some(1); 3], 5, 10, anywhere), None);
        assert_eq!(run(&[Some(1); 3], 1, 0, anywhere), None);
    }
}
