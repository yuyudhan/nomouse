//! The guided first run (feature NM-APP-10): walks the user through each permission nomouse
//! lacks, one at a time. For each: explain why it is needed and how to grant it, ask the OS
//! (its prompt, its settings pane, or a switch nomouse turns on itself), then wait until it is
//! granted. When everything is granted the caller learns whether to restart: a running nomouse
//! picks up a newly granted required permission only on a fresh start.
//!
//! [`Setup`] is a step machine over a [`Probe`] and an explicit clock, so it is tested with a
//! fake probe and fake times; [`guide`] drives it against a [`Presenter`] (a terminal or a
//! dialog). Each backend supplies its OS's permissions and probe.

use std::time::{Duration, Instant};

/// One permission, or system switch, nomouse needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    /// Stable id, such as `accessibility`.
    pub id: &'static str,
    /// The name the OS shows, such as `Accessibility`.
    pub name: &'static str,
    /// Why nomouse needs it.
    pub why: &'static str,
    /// What the user does to grant it.
    pub how: &'static str,
    /// nomouse cannot run without it: the daemon walks the user through it before starting,
    /// and restarts once it is granted. Optional ones (a switch the backend also turns on by
    /// itself) are only part of `nomouse setup`.
    pub required: bool,
}

/// How the flow observes and requests permissions on the OS.
pub trait Probe {
    /// Whether `permission` is granted right now.
    fn granted(&mut self, permission: &Permission) -> bool;

    /// Asks the OS for `permission`: shows its prompt, opens its settings pane, or turns a
    /// switch on.
    ///
    /// # Errors
    /// What went wrong, worded for the user (for example a command they must run as root).
    fn request(&mut self, permission: &Permission) -> Result<(), String>;
}

/// A probe for an OS where nomouse needs no permission: everything is granted.
#[derive(Debug, Clone, Copy, Default)]
pub struct NothingToAsk;

impl Probe for NothingToAsk {
    fn granted(&mut self, _permission: &Permission) -> bool {
        true
    }

    fn request(&mut self, _permission: &Permission) -> Result<(), String> {
        Ok(())
    }
}

/// What one step of the flow did, for the caller to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The permission was granted before the flow asked; nothing to do.
    AlreadyGranted(Permission),
    /// The permission is missing: show its `why` and `how` before the OS is asked.
    Explain(Permission),
    /// The OS was asked; the flow now waits for the grant.
    Requested(Permission),
    /// Still not granted; step again after [`Setup::POLL`].
    Waiting(Permission),
    /// Granted while the flow waited.
    Granted(Permission),
    /// Asking the OS failed; the reason says what the user can do. The flow has stopped.
    RequestFailed(Permission, String),
    /// Not granted before the timeout. The flow has stopped.
    TimedOut(Permission),
    /// Every permission is granted. `restart` is true when a required one was granted during
    /// the flow, so the process that ran it should restart to use it.
    Done {
        /// Whether a required permission was granted during the flow.
        restart: bool,
    },
}

impl Step {
    /// Whether the flow has ended: [`Step::Done`], [`Step::RequestFailed`] or
    /// [`Step::TimedOut`].
    pub const fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Done { .. } | Self::RequestFailed(..) | Self::TimedOut(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Check whether the current permission is granted.
    Check,
    /// It is missing and was explained; ask the OS.
    Ask,
    /// The OS was asked at `since`; wait for the grant.
    Wait { since: Instant },
}

/// The step machine. Call [`Setup::step`] until it returns a final step.
#[derive(Debug, Clone)]
pub struct Setup {
    permissions: Vec<Permission>,
    next: usize,
    phase: Phase,
    timeout: Duration,
    granted_during: bool,
    finished: Option<Step>,
}

impl Setup {
    /// How long to wait between checks while waiting for a grant.
    pub const POLL: Duration = Duration::from_millis(500);

    /// A flow over `permissions`, in order, giving each one `timeout` to be granted after the
    /// OS was asked.
    pub const fn new(permissions: Vec<Permission>, timeout: Duration) -> Self {
        Self {
            permissions,
            next: 0,
            phase: Phase::Check,
            timeout,
            granted_during: false,
            finished: None,
        }
    }

    /// Advances the flow at time `now`. A final step is returned again on every later call,
    /// and the probe is not asked anything more.
    pub fn step(&mut self, probe: &mut dyn Probe, now: Instant) -> Step {
        if let Some(finished) = &self.finished {
            return finished.clone();
        }
        let Some(&permission) = self.permissions.get(self.next) else {
            return self.finish(Step::Done {
                restart: self.granted_during,
            });
        };
        match self.phase {
            Phase::Check => {
                if probe.granted(&permission) {
                    self.advance();
                    Step::AlreadyGranted(permission)
                } else {
                    self.phase = Phase::Ask;
                    Step::Explain(permission)
                }
            }
            Phase::Ask => match probe.request(&permission) {
                Ok(()) => {
                    self.phase = Phase::Wait { since: now };
                    Step::Requested(permission)
                }
                Err(reason) => self.finish(Step::RequestFailed(permission, reason)),
            },
            Phase::Wait { since } => {
                if probe.granted(&permission) {
                    self.granted_during |= permission.required;
                    self.advance();
                    Step::Granted(permission)
                } else if now.saturating_duration_since(since) >= self.timeout {
                    self.finish(Step::TimedOut(permission))
                } else {
                    Step::Waiting(permission)
                }
            }
        }
    }

    /// Whether `permissions` are all granted, without asking for anything.
    pub fn all_granted(permissions: &[Permission], probe: &mut dyn Probe) -> bool {
        permissions
            .iter()
            .all(|permission| probe.granted(permission))
    }

    fn advance(&mut self) {
        self.next += 1;
        self.phase = Phase::Check;
    }

    fn finish(&mut self, step: Step) -> Step {
        self.finished = Some(step.clone());
        step
    }
}

/// Shows the flow to the user.
pub trait Presenter {
    /// Shows `step`. Returning `false` stops the flow (the user chose to quit); only
    /// [`Step::Explain`] needs a choice, other steps should return `true`.
    fn show(&mut self, step: &Step) -> bool;

    /// Waits `duration` before the next check, keeping a UI responsive meanwhile.
    fn wait(&mut self, duration: Duration);
}

/// How [`guide`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every permission is granted; restart when `restart` is true.
    Ready {
        /// Whether a required permission was granted during the flow.
        restart: bool,
    },
    /// The flow stopped at this step ([`Step::RequestFailed`] or [`Step::TimedOut`]).
    Stopped(Step),
    /// The user quit while being asked for this permission.
    Quit(Permission),
}

/// Runs the whole flow: steps `setup`, shows each step, waits [`Setup::POLL`] between checks.
pub fn guide(setup: &mut Setup, probe: &mut dyn Probe, presenter: &mut dyn Presenter) -> Outcome {
    loop {
        let step = setup.step(probe, Instant::now());
        if !presenter.show(&step)
            && let Step::Explain(permission) = step
        {
            return Outcome::Quit(permission);
        }
        match step {
            Step::Done { restart } => return Outcome::Ready { restart },
            Step::RequestFailed(..) | Step::TimedOut(_) => return Outcome::Stopped(step),
            Step::Waiting(_) => presenter.wait(Setup::POLL),
            Step::AlreadyGranted(_) | Step::Explain(_) | Step::Requested(_) | Step::Granted(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCESSIBILITY: Permission = Permission {
        id: "accessibility",
        name: "Accessibility",
        why: "read and press other apps' controls",
        how: "turn nomouse on in the Accessibility list",
        required: true,
    };
    const SWITCH: Permission = Permission {
        id: "switch",
        name: "Switch",
        why: "toolkits expose their trees",
        how: "nomouse turns it on",
        required: false,
    };

    /// A fake OS: each permission is granted after a number of checks (`None`: never), or as
    /// soon as it is requested (`on_request`); requests can fail.
    #[derive(Default)]
    struct FakeProbe {
        grants_after: Vec<(&'static str, Option<u32>)>,
        on_request: Vec<&'static str>,
        failing: Option<String>,
        checks: Vec<&'static str>,
        requests: Vec<&'static str>,
        granted_by_request: Vec<&'static str>,
    }

    impl Probe for FakeProbe {
        fn granted(&mut self, permission: &Permission) -> bool {
            self.checks.push(permission.id);
            if self.granted_by_request.contains(&permission.id) {
                return true;
            }
            let seen = self
                .checks
                .iter()
                .filter(|id| **id == permission.id)
                .count();
            self.grants_after
                .iter()
                .find(|(id, _)| *id == permission.id)
                .and_then(|(_, after)| *after)
                .is_some_and(|after| u32::try_from(seen).unwrap_or(u32::MAX) > after)
        }

        fn request(&mut self, permission: &Permission) -> Result<(), String> {
            self.requests.push(permission.id);
            if let Some(reason) = &self.failing {
                return Err(reason.clone());
            }
            if self.on_request.contains(&permission.id) {
                self.granted_by_request.push(permission.id);
            }
            Ok(())
        }
    }

    /// Steps until a final step, advancing a fake clock by [`Setup::POLL`] per step.
    fn run(setup: &mut Setup, probe: &mut FakeProbe) -> Vec<Step> {
        let start = Instant::now();
        let mut steps = Vec::new();
        for tick in 0..1000_u32 {
            let step = setup.step(probe, start + Setup::POLL * tick);
            let done = step.is_final();
            steps.push(step);
            if done {
                return steps;
            }
        }
        panic!("the flow did not end: {steps:?}");
    }

    #[test]
    fn everything_granted_needs_no_restart() {
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", Some(0)), ("switch", Some(0))],
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY, SWITCH], Duration::from_secs(60));
        assert_eq!(
            run(&mut setup, &mut probe),
            vec![
                Step::AlreadyGranted(ACCESSIBILITY),
                Step::AlreadyGranted(SWITCH),
                Step::Done { restart: false },
            ]
        );
        assert_eq!(probe.requests, Vec::<&str>::new());
    }

    #[test]
    fn a_missing_permission_is_explained_requested_once_and_waited_for() {
        // Granted on the fourth check: the first check plus two waiting polls fail.
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", Some(3))],
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY], Duration::from_secs(60));
        assert_eq!(
            run(&mut setup, &mut probe),
            vec![
                Step::Explain(ACCESSIBILITY),
                Step::Requested(ACCESSIBILITY),
                Step::Waiting(ACCESSIBILITY),
                Step::Waiting(ACCESSIBILITY),
                Step::Granted(ACCESSIBILITY),
                Step::Done { restart: true },
            ]
        );
        assert_eq!(probe.requests, vec!["accessibility"]);
    }

    #[test]
    fn a_switch_turned_on_by_the_request_is_granted_on_the_next_check() {
        let mut probe = FakeProbe {
            on_request: vec!["switch"],
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![SWITCH], Duration::from_secs(60));
        assert_eq!(
            run(&mut setup, &mut probe),
            vec![
                Step::Explain(SWITCH),
                Step::Requested(SWITCH),
                Step::Granted(SWITCH),
                Step::Done { restart: false },
            ]
        );
    }

    #[test]
    fn the_flow_stops_at_the_timeout_and_asks_nothing_more() {
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", None)],
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY, SWITCH], Setup::POLL * 3);
        let steps = run(&mut setup, &mut probe);
        assert_eq!(steps.last(), Some(&Step::TimedOut(ACCESSIBILITY)));
        assert_eq!(
            steps
                .iter()
                .filter(|step| matches!(step, Step::Waiting(_)))
                .count(),
            2
        );
        let checks = probe.checks.len();
        let later = Instant::now() + Duration::from_secs(3600);
        assert_eq!(setup.step(&mut probe, later), Step::TimedOut(ACCESSIBILITY));
        assert_eq!(probe.checks.len(), checks);
        assert_eq!(probe.requests, vec!["accessibility"]);
        assert!(!probe.checks.contains(&"switch"));
    }

    #[test]
    fn a_failed_request_stops_the_flow_with_its_reason() {
        let mut probe = FakeProbe {
            failing: Some("run the udev rule as root".to_owned()),
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![SWITCH, ACCESSIBILITY], Duration::from_secs(60));
        assert_eq!(
            run(&mut setup, &mut probe),
            vec![
                Step::Explain(SWITCH),
                Step::RequestFailed(SWITCH, "run the udev rule as root".to_owned()),
            ]
        );
        assert_eq!(probe.requests, vec!["switch"]);
    }

    #[test]
    fn permissions_are_handled_in_order() {
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", Some(1)), ("switch", Some(0))],
            ..FakeProbe::default()
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY, SWITCH], Duration::from_secs(60));
        assert_eq!(
            run(&mut setup, &mut probe),
            vec![
                Step::Explain(ACCESSIBILITY),
                Step::Requested(ACCESSIBILITY),
                Step::Granted(ACCESSIBILITY),
                Step::AlreadyGranted(SWITCH),
                Step::Done { restart: true },
            ]
        );
    }

    #[test]
    fn no_permissions_is_done_at_once() {
        let mut setup = Setup::new(Vec::new(), Duration::ZERO);
        assert_eq!(
            setup.step(&mut NothingToAsk, Instant::now()),
            Step::Done { restart: false }
        );
    }

    /// Records what was shown; answers Explain with `answer`.
    struct Recorder {
        shown: Vec<Step>,
        waits: u32,
        answer: bool,
    }

    impl Presenter for Recorder {
        fn show(&mut self, step: &Step) -> bool {
            self.shown.push(step.clone());
            !matches!(step, Step::Explain(_)) || self.answer
        }

        fn wait(&mut self, duration: Duration) {
            assert_eq!(duration, Setup::POLL);
            self.waits += 1;
        }
    }

    #[test]
    fn guide_waits_between_checks_and_reports_a_restart() {
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", Some(2))],
            ..FakeProbe::default()
        };
        let mut presenter = Recorder {
            shown: Vec::new(),
            waits: 0,
            answer: true,
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY], Duration::from_secs(60));
        let outcome = guide(&mut setup, &mut probe, &mut presenter);
        assert_eq!(outcome, Outcome::Ready { restart: true });
        assert_eq!(presenter.waits, 1);
        assert_eq!(presenter.shown.len(), 5);
    }

    #[test]
    fn guide_stops_when_the_user_quits_before_anything_is_requested() {
        let mut probe = FakeProbe::default();
        let mut presenter = Recorder {
            shown: Vec::new(),
            waits: 0,
            answer: false,
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY], Duration::from_secs(60));
        let outcome = guide(&mut setup, &mut probe, &mut presenter);
        assert_eq!(outcome, Outcome::Quit(ACCESSIBILITY));
        assert_eq!(probe.requests, Vec::<&str>::new());
    }

    #[test]
    fn guide_reports_where_it_stopped() {
        let mut probe = FakeProbe {
            grants_after: vec![("accessibility", None)],
            ..FakeProbe::default()
        };
        let mut presenter = Recorder {
            shown: Vec::new(),
            waits: 0,
            answer: true,
        };
        let mut setup = Setup::new(vec![ACCESSIBILITY], Duration::ZERO);
        let outcome = guide(&mut setup, &mut probe, &mut presenter);
        assert_eq!(outcome, Outcome::Stopped(Step::TimedOut(ACCESSIBILITY)));
    }
}
