//! Turning a refused plan request into a sentence a person can act on.
//!
//! The server's `error` is a stable machine code and its `message` is not - it is
//! prose that may be reworded. So the CLI keys on the code, says in its own words
//! what the code means for THIS command and what to do about it, and then shows
//! the server's message underneath as the detail (it names the serials, the first
//! differing resource, the policy that refused). The code itself stays visible in
//! both the human line and the `-o json` document, so a pipeline can match on it.
//!
//! The same code means different things to different commands: a `forbidden` on a
//! pull is about write access to state, on an approve it is about being an admin
//! or about not being a person at all. Hence [`PlanOp`].
//!
//! Pure: [`explain`] is a function of the error, tested against the response
//! shapes the spec documents.

use serde::Serialize;
use tm_api::ApiError;

/// Which plan command was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanOp {
    Push,
    Pull,
    Approve,
    /// `ls` / `show`: the review surface.
    Read,
}

/// A refusal, explained. Also the `-o json` shape of a failed plan command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explained {
    /// Always `false`: lets a consumer branch on one key whichever way it went.
    pub ok: bool,
    /// The stable machine code. The server's `error`, or - for a check the CLI
    /// made itself - the code the server uses for the same condition. `None` when
    /// the response carried none (a gateway's error page).
    pub error: Option<String>,
    /// HTTP status, when the refusal came from the server.
    pub status: Option<u16>,
    /// What it means, in plain words.
    pub explanation: String,
    /// What to do about it, a sentence per entry.
    pub hint: Vec<String>,
    /// The server's own `message` (or its `X-Terramantle-Refusal` header when the
    /// body had none). Detail for a person; not a contract.
    pub message: Option<String>,
    /// The process exit code this maps to.
    pub exit_code: i32,
}

impl Explained {
    /// A refusal the CLI reached on its own, before or after talking to the server.
    pub fn local(code: &str, explanation: impl Into<String>, hint: Vec<String>, exit: i32) -> Self {
        Explained {
            ok: false,
            error: Some(code.to_string()),
            status: None,
            explanation: explanation.into(),
            hint,
            message: None,
            exit_code: exit,
        }
    }

    /// The human rendering, one line per entry: the headline carries the code.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![match &self.error {
            Some(code) => format!("error: {} ({code})", self.explanation),
            None => format!("error: {}", self.explanation),
        }];
        out.extend(self.hint.iter().map(|h| format!("    {h}")));
        if let Some(m) = self.message.as_deref().filter(|m| !m.trim().is_empty()) {
            out.push(format!("    server: {m}"));
        }
        out
    }
}

fn s(text: &str) -> String {
    text.to_string()
}

/// Explain a refused request. `machine_token` says the caller is a CI or bot
/// identity, which changes what a `forbidden` on an approval means.
pub fn explain(op: PlanOp, e: &ApiError, machine_token: bool) -> Explained {
    let code = e.error_code();
    let status = e.status();
    // The header and the body's message are the same text when both are present;
    // the header is the fallback for a body a proxy replaced.
    let message = e
        .message()
        .or(e.refusal())
        .map(str::to_string)
        .filter(|m| !m.trim().is_empty());

    let (explanation, hint): (String, Vec<String>) = match (op, code, status) {
        // ── push ────────────────────────────────────────────────────────────
        (PlanOp::Push, Some("plan_render_mismatch"), _) => (
            s("the render and the binary are not the same plan"),
            vec![
                s("The gate judges the `show -json` render and a person reviews it, but apply runs the binary, so the server refuses a pair that does not describe the same changes or was not produced by the same terraform/tofu version."),
                s("Let `plan push` render the plan itself (drop --render), or re-run `show -json` on this exact plan file with the binary that made it. Nothing was stored."),
            ],
        ),
        (PlanOp::Push, Some("invalid_tfplan"), _) => (
            s("the server could not read this file as a saved plan"),
            vec![s("Push the file written by `plan -out=…`, unmodified. Nothing was stored.")],
        ),
        (PlanOp::Push, Some("plan_stale"), _) => (
            s("this plan is out of date: the workspace's state changed after it was made"),
            vec![
                s("It can no longer be applied, so the server did not store it. Run plan again against the current state and push that."),
                s("To keep this plan on record anyway, push it with --allow-stale; it still cannot be applied."),
            ],
        ),
        (PlanOp::Push, Some("lineage_mismatch"), _) => (
            s("this plan was made against a different state than the one this workspace holds"),
            vec![
                s("A lineage is a state's identity: the plan and the workspace are two different states, not two versions of one. --allow-stale does not change that."),
                s("Check the backend address resolves to the workspace you mean, then run plan again there. Nothing was stored."),
            ],
        ),
        (PlanOp::Push, Some("forbidden"), _) => (
            s("this token may not push plans to this workspace"),
            vec![s("Pushing a plan needs WRITE access to the workspace's state, the same access an apply needs.")],
        ),
        (PlanOp::Push, Some("payload_too_large"), _) => (
            s("the plan is too large to push"),
            vec![s("Limits: 100MB for a render pushed alone; with the binary, 50MB for the binary and 64MB for the two together.")],
        ),
        (PlanOp::Push, Some("quota_exceeded"), _) => (
            s("this org's subscription does not allow another plan push"),
            vec![s("The org is over a limit of its current tier; an owner can raise it under Billing in the portal.")],
        ),

        // ── pull ────────────────────────────────────────────────────────────
        (PlanOp::Pull, Some("forbidden"), _) => (
            s("this token may not pull plan binaries for this workspace: a pull needs WRITE access to its state"),
            vec![
                s("A plan binary holds the whole prior state and every variable value in clear, and the only thing it is for is applying - which needs write access anyway. Read access is not enough."),
                s("Run the pull with the identity that applies: the apply job's CI token, or a person with the member role or above."),
            ],
        ),
        (PlanOp::Pull, Some("policy_blocked"), _) => (
            s("the server refused to hand over this plan on policy grounds"),
            vec![
                s("A plan the gate blocked is served only once that exact version has been approved; an approval of another version, even of the same change, does not count."),
                s("Either approve it (`terramantle plan approve <workspace> <version-id> --reason …`) or push a plan that passes and pull that."),
                s("If the reason below says all state consumption is blocked for the org, that is the org's kill switch: only an owner lifting it helps."),
            ],
        ),
        (PlanOp::Pull, Some("plan_stale"), _) => (
            s("this plan can no longer be applied: the workspace's state changed after it was made"),
            vec![
                s("Applying it would be refused as stale. Run plan again, push it, and pull the new one."),
                s("To download it anyway, for inspection, pass --allow-stale."),
            ],
        ),
        (PlanOp::Pull, Some("plan_no_binary"), _) => (
            s("the newest plan here was pushed without its binary, so there is nothing to apply"),
            vec![s("The server deliberately does not fall back to an older plan. Push the plan with its binary (`terramantle plan push`, without --render-only) and pull again.")],
        ),
        (PlanOp::Pull, Some("not_found"), _) => (
            s("there is no plan to pull: nothing has been pushed for this workspace and ref, or the workspace or plan id does not exist"),
            vec![s("A plan pushed with --ref is pulled with the same --ref; without one, the pull addresses the plan that gates the workspace.")],
        ),

        // ── approve ─────────────────────────────────────────────────────────
        (PlanOp::Approve, Some("forbidden"), _) if machine_token => (
            s("a plan is approved by a person, and this is a machine identity"),
            vec![
                s("A CI or bot token cannot approve: the pipeline that pushed a plan must not be able to wave it through."),
                s("Sign in as yourself (`terramantle auth login`, with no TERRAMANTLE_TOKEN or client credentials in the environment) and approve from there, or use the portal."),
            ],
        ),
        (PlanOp::Approve, Some("forbidden"), _) => (
            s("your role in this org may not approve plans"),
            vec![s("Approving needs the admin role or above, and a human login. Ask an org admin to approve this plan.")],
        ),
        (PlanOp::Approve, Some("approver_role_too_low"), _) => (
            s("this org requires a higher role than yours to approve a plan"),
            vec![s("Nothing was recorded. Ask someone who holds the role named below.")],
        ),
        (PlanOp::Approve, Some("self_approval"), _) => (
            s("you pushed this plan, and this org does not let the person who pushed a plan approve it"),
            vec![s("Nothing was recorded. Another approver has to review and approve it.")],
        ),
        (PlanOp::Approve, Some("not_found"), _) => (
            s("no such workspace or plan version in this org"),
            vec![s("List the workspace's plans with `terramantle plan ls <workspace>` and use the full version id.")],
        ),

        // ── shared ──────────────────────────────────────────────────────────
        (_, Some("human_state_access_blocked"), _) => (
            s("this org does not let people use the state backend directly"),
            vec![s("A plan binary contains the state, so the same rule covers it. Run this from a CI pipeline or with a machine token.")],
        ),
        (_, Some("unauthorized" | "invalid_token"), _) => (
            s("not signed in, or the token has expired"),
            vec![s("Run `terramantle auth login`, or check the pipeline's credentials.")],
        ),
        (_, Some("forbidden"), _) => (
            s("this token is not allowed to do that in this org"),
            Vec::new(),
        ),
        (_, Some("not_found"), _) => (
            s("no such org, workspace or plan version"),
            Vec::new(),
        ),
        (_, Some("feature_disabled"), _) => (
            s("the state backend is not available to this org"),
            Vec::new(),
        ),
        (_, Some("bad_request"), _) => (s("the server rejected the request"), Vec::new()),
        (_, Some(_), _) => (s("the server refused the request"), Vec::new()),
        (_, None, Some(401)) => (
            s("not signed in, or the token has expired"),
            vec![s("Run `terramantle auth login`, or check the pipeline's credentials.")],
        ),
        (_, None, Some(403)) => (s("access was refused"), Vec::new()),
        (_, None, Some(st)) => (format!("the server answered HTTP {st}"), Vec::new()),
        // Transport, body or JSON failures: the error's own Display says it all.
        (_, None, None) => (e.to_string(), Vec::new()),
    };

    Explained {
        ok: false,
        error: code.map(str::to_string),
        status,
        explanation,
        hint,
        message,
        exit_code: e.exit_code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(http: u16, body: &str) -> ApiError {
        ApiError::from_status_with_refusal(http, "https://x/y".into(), body.into(), None)
    }

    fn coded(http: u16, code: &str, message: &str) -> ApiError {
        err(
            http,
            &serde_json::json!({ "error": code, "message": message }).to_string(),
        )
    }

    /// Every code the spec documents for `pushPlan`, at its documented status.
    #[test]
    fn push_refusals_each_get_their_own_sentence_and_exit() {
        let cases = [
            (400, "plan_render_mismatch", 2, "not the same plan"),
            (400, "invalid_tfplan", 2, "could not read this file"),
            (409, "plan_stale", 7, "out of date"),
            (409, "lineage_mismatch", 7, "different state"),
            (403, "forbidden", 5, "may not push plans"),
            (403, "human_state_access_blocked", 5, "does not let people"),
            (413, "payload_too_large", 1, "too large"),
            (402, "quota_exceeded", 1, "subscription"),
            (404, "not_found", 6, "no such org"),
        ];
        let mut seen = std::collections::HashSet::new();
        for (http, code, exit, needle) in cases {
            let x = explain(PlanOp::Push, &coded(http, code, "server words"), false);
            assert_eq!(x.exit_code, exit, "{code}");
            assert_eq!(x.error.as_deref(), Some(code));
            assert_eq!(x.status, Some(http));
            assert!(x.explanation.contains(needle), "{code}: {}", x.explanation);
            assert_eq!(x.message.as_deref(), Some("server words"));
            assert!(
                seen.insert(x.explanation.clone()),
                "{code} shares a sentence"
            );
        }
    }

    /// Stale and lineage-mismatch are both exit 7 and must still read differently:
    /// only one of them is fixed by re-planning in place.
    #[test]
    fn stale_and_lineage_mismatch_give_different_advice() {
        let stale = explain(PlanOp::Push, &coded(409, "plan_stale", "m"), false);
        let lineage = explain(PlanOp::Push, &coded(409, "lineage_mismatch", "m"), false);
        assert!(stale.hint.join(" ").contains("--allow-stale"));
        assert!(lineage.hint.join(" ").contains("does not change that"));
        assert!(lineage.hint.join(" ").contains("backend address"));
    }

    #[test]
    fn pull_refusals_are_explained_for_a_pull() {
        // 403: the one that is NOT "you are not signed in" - it is about write.
        let forbidden = explain(
            PlanOp::Pull,
            &coded(
                403,
                "forbidden",
                "Pulling a binary plan needs write access to state for this workspace",
            ),
            true,
        );
        assert_eq!(forbidden.exit_code, 5);
        assert!(forbidden.explanation.contains("WRITE access"));
        assert!(forbidden
            .hint
            .join(" ")
            .contains("Read access is not enough"));

        let blocked = explain(
            PlanOp::Pull,
            &coded(423, "policy_blocked", "plan p1 is blocked by policy and has not been approved: 3 destroys exceed max_destroys 1"),
            true,
        );
        assert_eq!(blocked.exit_code, 3);
        assert!(blocked.explanation.contains("policy grounds"));
        assert!(blocked.hint.join(" ").contains("plan approve"));
        assert!(blocked.message.as_deref().unwrap().contains("3 destroys"));

        let stale = explain(PlanOp::Pull, &coded(409, "plan_stale", "m"), true);
        assert_eq!(stale.exit_code, 7);
        assert!(stale.explanation.contains("can no longer be applied"));

        let no_binary = explain(PlanOp::Pull, &coded(404, "plan_no_binary", "m"), true);
        assert_eq!(no_binary.exit_code, 6);
        assert!(no_binary.explanation.contains("without its binary"));

        let missing = explain(PlanOp::Pull, &coded(404, "not_found", "m"), true);
        assert_eq!(missing.exit_code, 6);
        assert!(missing.hint.join(" ").contains("--ref"));
    }

    /// The reason survives a body that is not the worker's JSON, via the header.
    #[test]
    fn the_refusal_header_stands_in_for_a_missing_body_message() {
        let e = ApiError::from_status_with_refusal(
            423,
            "u".into(),
            "<html>locked</html>".into(),
            Some("plan p1 is blocked by policy and has not been approved: denied".into()),
        );
        let x = explain(PlanOp::Pull, &e, true);
        assert_eq!(x.error, None);
        assert!(x.message.as_deref().unwrap().contains("denied"));
        assert!(x.lines().iter().any(|l| l.contains("server: plan p1")));
        // And when both exist, the body's message is the one shown, once.
        let both = ApiError::from_status_with_refusal(
            423,
            "u".into(),
            r#"{"error":"policy_blocked","message":"body words"}"#.into(),
            Some("header words".into()),
        );
        let x = explain(PlanOp::Pull, &both, true);
        assert_eq!(x.message.as_deref(), Some("body words"));
    }

    #[test]
    fn approval_refusals_map_to_plain_sentences() {
        let human_only = "This action must be performed by a human user";
        let machine = explain(PlanOp::Approve, &coded(403, "forbidden", human_only), true);
        assert!(machine.explanation.contains("machine identity"));
        assert!(machine.hint.join(" ").contains("auth login"));
        assert_eq!(machine.exit_code, 5);

        // The same code from a person is about their role, not about being a bot.
        let person = explain(
            PlanOp::Approve,
            &coded(403, "forbidden", "Requires admin role"),
            false,
        );
        assert!(person.explanation.contains("role"));
        assert!(!person.explanation.contains("machine"));

        let low = explain(
            PlanOp::Approve,
            &coded(
                403,
                "approver_role_too_low",
                "Approving a plan in this org needs the owner role or above",
            ),
            false,
        );
        assert_eq!(low.exit_code, 5);
        assert!(low.explanation.contains("higher role"));
        assert!(low.message.as_deref().unwrap().contains("owner"));

        let own = explain(PlanOp::Approve, &coded(403, "self_approval", "m"), false);
        assert_eq!(own.exit_code, 5);
        assert!(own.explanation.contains("you pushed this plan"));

        let gone = explain(PlanOp::Approve, &coded(404, "not_found", "m"), false);
        assert_eq!(gone.exit_code, 6);
        assert!(gone.hint.join(" ").contains("plan ls"));
    }

    /// The machine code is in the human headline AND the JSON document.
    #[test]
    fn the_stable_code_is_visible_in_both_renderings() {
        let x = explain(
            PlanOp::Push,
            &coded(409, "plan_stale", "serial 4 vs 6"),
            false,
        );
        let lines = x.lines();
        assert!(lines[0].starts_with("error: "));
        assert!(lines[0].ends_with("(plan_stale)"));
        assert!(lines.last().unwrap().contains("server: serial 4 vs 6"));

        let json = serde_json::to_value(&x).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "plan_stale");
        assert_eq!(json["status"], 409);
        assert_eq!(json["exit_code"], 7);
        assert_eq!(json["message"], "serial 4 vs 6");
        assert!(json["explanation"]
            .as_str()
            .unwrap()
            .contains("out of date"));
    }

    #[test]
    fn unknown_and_codeless_responses_still_say_something() {
        let odd = explain(PlanOp::Push, &coded(500, "upstream_error", "boom"), false);
        assert_eq!(odd.exit_code, 1);
        assert_eq!(odd.error.as_deref(), Some("upstream_error"));
        assert_eq!(odd.message.as_deref(), Some("boom"));

        let gateway = explain(PlanOp::Pull, &err(502, "<html>bad gateway</html>"), false);
        assert_eq!(gateway.error, None);
        assert!(gateway.explanation.contains("502"));
        assert!(gateway.lines()[0].starts_with("error: the server answered"));

        let unauth = explain(PlanOp::Read, &err(401, ""), false);
        assert_eq!(unauth.exit_code, 5);
        assert!(unauth.hint.join(" ").contains("auth login"));
    }

    #[test]
    fn a_local_refusal_has_the_same_shape() {
        let x = Explained::local("plan_stale", "out of date", vec!["re-plan".into()], 7);
        assert_eq!(x.status, None);
        assert_eq!(
            x.lines(),
            vec![
                "error: out of date (plan_stale)".to_string(),
                "    re-plan".to_string()
            ]
        );
    }
}
