//! Property tests of the guided first run: for any set of permissions and any OS behaviour,
//! the flow ends, asks the OS at most once per permission, and finishes `Done` exactly when
//! every permission was granted in time, with a restart exactly when a required one was
//! missing.

use std::time::Instant;

use nomouse_core::setup::{Permission, Probe, Setup, Step};
use proptest::prelude::*;

const IDS: [&str; 4] = ["accessibility", "input-monitoring", "a11y-switch", "uinput"];

fn permission(id: &'static str) -> Permission {
    Permission {
        id,
        name: id,
        why: "needed",
        how: "grant it",
        // The first two are required, the others optional switches.
        required: IDS.iter().position(|known| *known == id) < Some(2),
    }
}

/// How the fake OS treats one permission.
#[derive(Debug, Clone, Copy)]
struct Behaviour {
    /// Granted before the flow starts.
    initially: bool,
    /// Granted on this check after the request (0: the first one); `None` never.
    after_request: Option<u32>,
    /// The request fails.
    request_fails: bool,
}

fn behaviour() -> impl Strategy<Value = Behaviour> {
    (
        any::<bool>(),
        proptest::option::of(0u32..20),
        proptest::bool::weighted(0.15),
    )
        .prop_map(|(initially, after_request, request_fails)| Behaviour {
            initially,
            after_request,
            request_fails,
        })
}

struct FakeOs {
    behaviour: Vec<Behaviour>,
    requested: Vec<u32>,
    checks_since_request: Vec<u32>,
}

impl FakeOs {
    fn index(id: &str) -> usize {
        IDS.iter().position(|known| *known == id).unwrap_or(0)
    }
}

impl Probe for FakeOs {
    fn granted(&mut self, permission: &Permission) -> bool {
        let index = Self::index(permission.id);
        let behaviour = self.behaviour[index];
        if behaviour.initially {
            return true;
        }
        if self.requested[index] == 0 {
            return false;
        }
        let seen = self.checks_since_request[index];
        self.checks_since_request[index] += 1;
        behaviour.after_request.is_some_and(|after| seen >= after)
    }

    fn request(&mut self, permission: &Permission) -> Result<(), String> {
        let index = Self::index(permission.id);
        self.requested[index] += 1;
        if self.behaviour[index].request_fails {
            Err("cannot".to_owned())
        } else {
            Ok(())
        }
    }
}

proptest! {
    #[test]
    fn the_flow_ends_asks_once_and_reports_truthfully(
        behaviours in proptest::collection::vec(behaviour(), 0..=IDS.len()),
        polls in 1u32..30,
    ) {
        let permissions: Vec<Permission> =
            IDS.iter().take(behaviours.len()).map(|id| permission(id)).collect();
        let mut os = FakeOs {
            behaviour: behaviours.clone(),
            requested: vec![0; behaviours.len()],
            checks_since_request: vec![0; behaviours.len()],
        };
        let timeout = Setup::POLL * polls;
        let mut setup = Setup::new(permissions, timeout);
        let start = Instant::now();
        // Each permission takes at most: check, request, `polls` waits, the final check.
        let bound = behaviours.len() * (usize::try_from(polls).unwrap_or(0) + 3) + 1;
        let mut last = None;
        for tick in 0..=u32::try_from(bound).unwrap_or(u32::MAX) {
            let step = setup.step(&mut os, start + Setup::POLL * tick);
            if step.is_final() {
                last = Some(step);
                break;
            }
        }
        let last = last.expect("the flow ended within its bound");
        prop_assert!(os.requested.iter().all(|count| *count <= 1));

        // Walk the permissions in order to predict the end.
        let mut expected = Step::Done { restart: false };
        let mut restart = false;
        for (index, behaviour) in behaviours.iter().enumerate() {
            let permission = permission(IDS[index]);
            if behaviour.initially {
                continue;
            }
            if behaviour.request_fails {
                expected = Step::RequestFailed(permission, "cannot".to_owned());
                break;
            }
            // The j-th check after the request (0-based) comes (j + 1) polls after it, and the
            // flow gives up at the first unanswered check at or past the timeout.
            match behaviour.after_request {
                Some(after) if Setup::POLL * (after + 1) <= timeout => restart |= permission.required,
                _ => {
                    expected = Step::TimedOut(permission);
                    break;
                }
            }
        }
        if let Step::Done { .. } = expected {
            expected = Step::Done { restart };
        }
        prop_assert_eq!(last, expected);
    }
}
