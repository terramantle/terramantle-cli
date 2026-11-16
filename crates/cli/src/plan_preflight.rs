//! The local pre-flight for `plan push`: can this plan still be applied to the
//! workspace's current state?
//!
//! The server asks the same question when the plan arrives and refuses a binary
//! that fails it. Asking it here first buys two things. A 50MB upload that was
//! always going to be refused is not made. And the answer can be explained: the
//! server's refusal is one sentence in a 409, while a pipeline author needs to
//! know what happened and what to run next.
//!
//! The rules are the server's PUSH rules, exactly - the lineage guard in
//! `PlanService.pushPlan` and `planStaleReason` beside it - because a pre-flight
//! that is stricter than the server refuses plans the server would have taken,
//! and one that is laxer explains nothing. A state is identified by the pair
//! (lineage, serial): lineage is its identity, minted once at the first apply;
//! serial counts its writes.
//!
//! * The workspace has NO state: every plan is current. Plans are pushed for a
//!   first apply, and for workspaces whose state lives somewhere else (the
//!   passive-drift recipe), so there is nothing here for a plan to be behind.
//! * The workspace HAS state:
//!   - a plan whose lineage differs from the workspace's is a `lineage_mismatch`;
//!   - a plan with NO lineage (made against an empty state) is stale;
//!   - a plan whose serial differs from the workspace's is stale.
//!
//! The no-lineage rule is the one terraform and tofu cannot see for themselves:
//! they stale-check a saved plan at apply time only when the plan has a lineage,
//! so a plan made against an empty state applies happily to a workspace that has
//! since gained one - and tries to create everything again.
//!
//! This is NOT the "require a passing plan" freshness rule, which the server
//! applies at LOCK and which needs a state to match: that one decides whether an
//! apply may start, this one only whether a push will be accepted.
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
}

/// Compare the plan's prior state with the workspace's current one.
/// `current` is `None` when the workspace has no state (or does not exist yet).
///
/// The order is the server's: the lineage guard runs first and answers
/// `lineage_mismatch`; staleness is asked only of a plan that survived it.
pub fn check(plan: &PlanPrior, current: Option<&StatePointer>) -> Preflight {
    // No state: never stale, and nothing to mismatch. Whatever the plan was
    // made from, the server stores it.
    let Some(current) = current else {
        return Preflight::Current;
    };
    // Lineage guard: both known, and different.
    if let (Some(p), Some(c)) = (&plan.lineage, &current.lineage) {
        if p != c {
            return Preflight::LineageMismatch {
                plan: p.clone(),
                current: c.clone(),
            };
        }
    }
    // A binary with no lineage, for a workspace whose state has one.
    if plan.lineage.is_none() && current.lineage.is_some() {
        return Preflight::PredatesState {
            current: current.serial,
        };
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
            Preflight::LineageMismatch { .. } => Some("lineage_mismatch"),
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

    /// A workspace with no state is never stale and never a mismatch, WHATEVER
    /// the plan was made from. The server's `planStaleReason` returns null as
    /// soon as there is no current serial, and its lineage guard needs a current
    /// lineage to compare. Refusing a lineage-bearing plan here (as this did
    /// before) broke the passive-drift recipe: a workspace whose state lives
    /// elsewhere has plans with a lineage and no state on the server.
    #[test]
    fn without_state_every_plan_is_current() {
        // A first apply: made against nothing, whatever serial a new state
        // starts at in this release.
        assert_eq!(check(&plan(None, Some(0)), None), Preflight::Current);
        assert_eq!(check(&plan(None, Some(1)), None), Preflight::Current);
        assert_eq!(check(&plan(None, None), None), Preflight::Current);
        // State that lives somewhere else: a lineage and a serial, and still current.
        assert_eq!(check(&plan(Some("L"), Some(4)), None), Preflight::Current);
        assert_eq!(check(&plan(Some("L"), None), None), Preflight::Current);
    }

    /// The server's `planStaleReason`, case for case, as a table: the pre-flight
    /// must give the same answer the push would. `tfplan` is the lineage source
    /// throughout, since the pre-flight only ever runs for a binary.
    #[test]
    fn the_answer_is_the_servers_for_every_combination() {
        // (plan lineage, plan serial, workspace state, expected machine code)
        type Case = (
            Option<&'static str>,
            Option<i64>,
            Option<(Option<&'static str>, i64)>,
            Option<&'static str>,
        );
        let cases: &[Case] = &[
            // current.serial == null  ->  null
            (None, Some(0), None, None),
            (Some("L"), Some(7), None, None),
            // lineage guard: both known and different -> lineage_mismatch (before staleness)
            (
                Some("X"),
                Some(5),
                Some((Some("L"), 5)),
                Some("lineage_mismatch"),
            ),
            (
                Some("X"),
                Some(1),
                Some((Some("L"), 5)),
                Some("lineage_mismatch"),
            ),
            // tfplan && prior_lineage == null && current.lineage != null -> stale
            (None, Some(0), Some((Some("L"), 3)), Some("plan_stale")),
            (None, Some(3), Some((Some("L"), 3)), Some("plan_stale")),
            // ...but not when the workspace's own lineage is unknown: then only the serial is compared
            (None, Some(3), Some((None, 3)), None),
            (None, Some(2), Some((None, 3)), Some("plan_stale")),
            // prior_serial != null && prior_serial !== current.serial -> stale
            (Some("L"), Some(4), Some((Some("L"), 5)), Some("plan_stale")),
            (Some("L"), Some(6), Some((Some("L"), 5)), Some("plan_stale")),
            (Some("L"), None, Some((Some("L"), 5)), None),
            // otherwise -> null
            (Some("L"), Some(5), Some((Some("L"), 5)), None),
            (Some("L"), Some(5), Some((None, 5)), None),
        ];
        for (lineage, serial, ws, want) in cases {
            let ws = ws.map(|(l, s)| state(l, s));
            let got = check(&plan(*lineage, *serial), ws.as_ref());
            assert_eq!(
                got.code(),
                *want,
                "plan ({lineage:?}, {serial:?}) against {ws:?} gave {got:?}"
            );
        }
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
