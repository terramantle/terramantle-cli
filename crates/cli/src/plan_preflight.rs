//! The local pre-flight for `plan push`: can this plan still be applied to the
//! workspace's current state?
//!
//! The server asks the same question when the plan arrives and refuses a binary
//! that fails it. Asking it here first buys two things. A 50MB upload that was
//! always going to be refused is not made. And the answer can be explained: the
//! server's refusal is one sentence in a 409, while a pipeline author needs to
//! know what happened and what to run next.
//!
//! A state is identified by the PAIR (lineage, serial). Lineage is the state's
//! identity, minted once at the first apply; serial counts its writes. A plan is
//! current only if it was computed from exactly the pair the workspace holds now:
//!
//! * the workspace HAS state: the plan's lineage and serial must both equal it;
//! * the workspace has NO state: only a plan made against no state (no lineage)
//!   is current.
//!
//! The second half is the one terraform and tofu cannot see for themselves: they
//! stale-check a saved plan at apply time only when the plan has a lineage, so a
//! plan made against an empty state applies happily to a workspace that has since
//! gained one - and tries to create everything again.
//!
//! Pure: [`check`] compares two values and the text is built from the result, so
//! every rule and every sentence is tested with no network.

use crate::tfplan_zip::PlanPrior;

/// The workspace's current state: the newest state version's serial and lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePointer {
    pub serial: i64,
    /// `None` only for a state version stored without a lineage, in which case
    /// lineage cannot be compared and the serial alone decides.
    pub lineage: Option<String>,
}

/// The pre-flight's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    /// The plan was computed from the state the workspace holds now.
    Current,
    /// Same state, older (or newer) write: something changed it after the plan.
    StaleSerial { plan: i64, current: i64 },
    /// The plan was made against no state and the workspace has state now.
    PredatesState { current: i64 },
    /// The plan was made against a different state altogether.
    LineageMismatch { plan: String, current: String },
    /// The plan was made against some state and the workspace has none.
    NoStateForLineage { plan: String },
}

/// Compare the plan's prior state with the workspace's current one.
/// `current` is `None` when the workspace has no state (or does not exist yet).
pub fn check(plan: &PlanPrior, current: Option<&StatePointer>) -> Preflight {
    let Some(current) = current else {
        return match &plan.lineage {
            None => Preflight::Current,
            Some(l) => Preflight::NoStateForLineage { plan: l.clone() },
        };
    };
    match (&plan.lineage, &current.lineage) {
        (None, _) => {
            return Preflight::PredatesState {
                current: current.serial,
            }
        }
        (Some(p), Some(c)) if p != c => {
            return Preflight::LineageMismatch {
                plan: p.clone(),
                current: c.clone(),
            }
        }
        _ => {}
    }
    match plan.serial {
        Some(s) if s != current.serial => Preflight::StaleSerial {
            plan: s,
            current: current.serial,
        },
        // No serial in the plan's state: nothing to compare, and the server does
        // not refuse on it either.
        _ => Preflight::Current,
    }
}

impl Preflight {
    /// The stable machine code - the one the server answers with for the same
    /// condition, so a pipeline matches one string whichever side noticed.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Preflight::Current => None,
            Preflight::StaleSerial { .. } | Preflight::PredatesState { .. } => Some("plan_stale"),
            Preflight::LineageMismatch { .. } | Preflight::NoStateForLineage { .. } => {
                Some("lineage_mismatch")
            }
        }
    }

    /// One line: what is wrong.
    pub fn headline(&self) -> Option<&'static str> {
        Some(match self {
            Preflight::Current => return None,
            Preflight::StaleSerial { .. } => {
                "this plan is out of date: the workspace's state changed after it was made"
            }
            Preflight::PredatesState { .. } => {
                "this plan was made before the workspace had any state"
            }
            Preflight::LineageMismatch { .. } => {
                "this plan was made against a different state than the one this workspace holds"
            }
            Preflight::NoStateForLineage { .. } => {
                "this plan was made against a state this workspace does not have"
            }
        })
    }

    /// What happened and what to do, a sentence per line. `tf` is the binary's
    /// name, so the instruction reads `tofu plan` to a tofu user.
    pub fn explain(&self, workspace: &str, tf: &str) -> Vec<String> {
        let replan = format!(
            "Run `{tf} plan -out=tfplan` again, so the plan is made from the current state, and push that."
        );
        match self {
            Preflight::Current => Vec::new(),
            Preflight::StaleSerial { plan, current } => vec![
                format!(
                    "The plan was computed from state serial {plan}; workspace `{workspace}` is now at serial {current}."
                ),
                if plan < current {
                    "Something was applied, or the state was otherwise written, after this plan was made."
                        .to_string()
                } else {
                    "The workspace is at an EARLIER serial than the plan, which usually means its state was rolled back or the plan came from another copy of it."
                        .to_string()
                },
                format!("`{tf} apply` would refuse this plan as stale, so it cannot gate or be applied."),
                replan,
            ],
            Preflight::PredatesState { current } => vec![
                format!(
                    "The plan was computed from an empty state; workspace `{workspace}` now has state, at serial {current}."
                ),
                format!(
                    "`{tf} apply` does not check a plan like this - it would try to create everything the state already tracks."
                ),
                replan,
            ],
            Preflight::LineageMismatch { plan, current } => vec![
                format!("The plan's state has lineage {plan}; workspace `{workspace}` holds lineage {current}."),
                "A lineage is a state's identity, so these are two different states, not two versions of one."
                    .to_string(),
                "Usual causes: the plan was made in another stack or against another backend, the backend address resolves to the wrong workspace, or this workspace's state was replaced."
                    .to_string(),
                format!("Check the backend address, then: {}", lowercase_first(&replan)),
            ],
            Preflight::NoStateForLineage { plan } => vec![
                format!("The plan's state has lineage {plan}; workspace `{workspace}` has no state at all."),
                "So the plan was computed from a state that lives somewhere else: another backend, another workspace, or a local state file."
                    .to_string(),
                format!(
                    "`{tf} apply` would refuse it against this backend. Check the backend address, then: {}",
                    lowercase_first(&replan)
                ),
            ],
        }
    }
}

fn lowercase_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(lineage: Option<&str>, serial: Option<i64>) -> PlanPrior {
        PlanPrior {
            serial,
            lineage: lineage.map(str::to_string),
            terraform_version: None,
        }
    }

    fn state(lineage: Option<&str>, serial: i64) -> StatePointer {
        StatePointer {
            serial,
            lineage: lineage.map(str::to_string),
        }
    }

    #[test]
    fn with_state_both_lineage_and_serial_must_match() {
        let ws = state(Some("L"), 5);
        assert_eq!(
            check(&plan(Some("L"), Some(5)), Some(&ws)),
            Preflight::Current
        );
        assert_eq!(
            check(&plan(Some("L"), Some(4)), Some(&ws)),
            Preflight::StaleSerial {
                plan: 4,
                current: 5
            }
        );
        // Ahead is as wrong as behind: it is not the state the workspace holds.
        assert_eq!(
            check(&plan(Some("L"), Some(6)), Some(&ws)),
            Preflight::StaleSerial {
                plan: 6,
                current: 5
            }
        );
        // A matching serial does not rescue a foreign lineage, and lineage is
        // reported first: re-planning against the wrong state would not help.
        assert_eq!(
            check(&plan(Some("X"), Some(5)), Some(&ws)),
            Preflight::LineageMismatch {
                plan: "X".into(),
                current: "L".into()
            }
        );
        assert_eq!(
            check(&plan(Some("X"), Some(1)), Some(&ws)),
            Preflight::LineageMismatch {
                plan: "X".into(),
                current: "L".into()
            }
        );
    }

    /// The case no client checks: an empty-state plan against a workspace that
    /// has state. It is stale EVEN WHEN the serials happen to be equal, which is
    /// exactly when a serial comparison alone would wave it through.
    #[test]
    fn a_plan_with_no_lineage_is_stale_once_the_workspace_has_state() {
        assert_eq!(
            check(&plan(None, Some(0)), Some(&state(Some("L"), 3))),
            Preflight::PredatesState { current: 3 }
        );
        assert_eq!(
            check(&plan(None, Some(1)), Some(&state(Some("L"), 1))),
            Preflight::PredatesState { current: 1 }
        );
    }

    #[test]
    fn without_state_only_a_plan_with_no_lineage_is_current() {
        assert_eq!(check(&plan(None, Some(0)), None), Preflight::Current);
        // Whatever serial a new state starts at in this release.
        assert_eq!(check(&plan(None, Some(1)), None), Preflight::Current);
        assert_eq!(check(&plan(None, None), None), Preflight::Current);
        assert_eq!(
            check(&plan(Some("L"), Some(4)), None),
            Preflight::NoStateForLineage { plan: "L".into() }
        );
    }

    /// What cannot be compared is not a reason to refuse: the server does not
    /// refuse on it either, and the pre-flight must never be stricter by accident.
    #[test]
    fn unknowns_are_not_mismatches() {
        // A state version stored with no lineage: the serial decides.
        assert_eq!(
            check(&plan(Some("L"), Some(2)), Some(&state(None, 2))),
            Preflight::Current
        );
        assert_eq!(
            check(&plan(Some("L"), Some(1)), Some(&state(None, 2))),
            Preflight::StaleSerial {
                plan: 1,
                current: 2
            }
        );
        // A plan whose state has no integer serial.
        assert_eq!(
            check(&plan(Some("L"), None), Some(&state(Some("L"), 9))),
            Preflight::Current
        );
    }

    #[test]
    fn codes_are_the_servers() {
        assert_eq!(Preflight::Current.code(), None);
        assert_eq!(
            Preflight::StaleSerial {
                plan: 1,
                current: 2
            }
            .code(),
            Some("plan_stale")
        );
        assert_eq!(
            Preflight::PredatesState { current: 2 }.code(),
            Some("plan_stale")
        );
        assert_eq!(
            Preflight::LineageMismatch {
                plan: "a".into(),
                current: "b".into()
            }
            .code(),
            Some("lineage_mismatch")
        );
        assert_eq!(
            Preflight::NoStateForLineage { plan: "a".into() }.code(),
            Some("lineage_mismatch")
        );
    }

    /// A stale serial and a lineage mismatch must not read alike: one is fixed by
    /// re-planning, the other by finding out why the plan is pointed at the wrong
    /// state.
    #[test]
    fn stale_and_mismatch_are_explained_differently() {
        let stale = Preflight::StaleSerial {
            plan: 4,
            current: 6,
        };
        let text = stale.explain("prod-network", "tofu").join("\n");
        assert!(text.contains("serial 4"));
        assert!(text.contains("serial 6"));
        assert!(text.contains("prod-network"));
        assert!(text.contains("tofu plan -out=tfplan"));
        assert!(!text.contains("lineage"));

        let mismatch = Preflight::LineageMismatch {
            plan: "aaaa".into(),
            current: "bbbb".into(),
        };
        let text = mismatch.explain("prod-network", "terraform").join("\n");
        assert!(text.contains("aaaa") && text.contains("bbbb"));
        assert!(text.contains("two different states"));
        assert!(text.contains("backend address"));
        assert!(text.contains("terraform plan -out=tfplan"));
        assert_ne!(stale.headline(), mismatch.headline());
    }

    #[test]
    fn every_refusal_says_what_to_do() {
        for p in [
            Preflight::StaleSerial {
                plan: 1,
                current: 2,
            },
            Preflight::StaleSerial {
                plan: 3,
                current: 2,
            },
            Preflight::PredatesState { current: 2 },
            Preflight::LineageMismatch {
                plan: "a".into(),
                current: "b".into(),
            },
            Preflight::NoStateForLineage { plan: "a".into() },
        ] {
            assert!(p.code().is_some());
            assert!(p.headline().is_some());
            let text = p.explain("ws", "tofu").join(" ");
            assert!(
                text.contains("plan -out=tfplan"),
                "{p:?} gives no next step"
            );
        }
        assert!(Preflight::Current.explain("ws", "tofu").is_empty());
        assert_eq!(Preflight::Current.headline(), None);
    }

    /// A rollback reads differently from an ordinary stale plan.
    #[test]
    fn a_plan_ahead_of_the_workspace_is_called_out() {
        let ahead = Preflight::StaleSerial {
            plan: 9,
            current: 7,
        };
        assert!(ahead.explain("ws", "tofu").join(" ").contains("EARLIER"));
    }
}
