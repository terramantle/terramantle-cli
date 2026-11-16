//! How a plan's verdict and its review detail are printed.
//!
//! Everything here is a pure function from the server's response shapes to text,
//! so what a pipeline log or a terminal shows is pinned by tests that need no
//! server: the push verdict (`plan push`), the review page (`plan show`) and the
//! result of an approval (`plan approve`).
//!
//! One rule runs through all of it: there are three levels, and only one of them
//! stops anything. An advisory policy or a limit in `warn` mode is WARN. It is
//! never printed as a block, because the server never treats it as one.

use tm_api::generated::models::{
    ApprovePlanVersionResponse, GetPlanVersionResponse, GetPlanVersionResponseState,
    ListPlanVersionApprovalsResponse, PlanImpact, PlanImpactGraph, PlanIndicator,
    PlanIndicatorMode, PlanIndicatorStatus, PlanPolicyResult, PlanProvenance, PlanPushResult,
    PlanRun, PlanSupplyChain, PlanSupplyChainModulesListItem, PlanSupplyChainModulesListItemOrigin,
    PlanSupplyChainProvidersListItem, PlanSupplyChainSource, PlanVersionSummary,
};

use crate::output::{neutral_mark, relative_time, Gate, Style, TableView, DASH};

/// How many addresses / names a list shows before it says "+N more".
const SAMPLE: usize = 8;

// ── numbers ─────────────────────────────────────────────────────────────────

/// An indicator value: an integer when it is one, one decimal otherwise.
fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

fn with_unit(v: f64, unit: Option<&str>) -> String {
    format!("{}{}", num(v), unit.unwrap_or(""))
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `a, b, c (+2 more)`, where `total` is the real count behind a sampled list.
fn sample(items: &[String], total: Option<i64>) -> String {
    let shown: Vec<&str> = items.iter().take(SAMPLE).map(String::as_str).collect();
    let total = total.unwrap_or(items.len() as i64).max(items.len() as i64);
    let more = total - shown.len() as i64;
    if more > 0 {
        format!("{} (+{more} more)", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

// ── indicators ──────────────────────────────────────────────────────────────

/// An indicator's level. `None` when it carries no judgement: shown for context
/// (`mode: info`), or not measurable for this plan.
pub fn indicator_gate(i: &PlanIndicator) -> Option<Gate> {
    match i.status? {
        PlanIndicatorStatus::Block => Some(Gate::Blocked),
        PlanIndicatorStatus::Warn => Some(Gate::Warn),
        PlanIndicatorStatus::Ok if i.mode != Some(PlanIndicatorMode::Info) => Some(Gate::Pass),
        PlanIndicatorStatus::Ok | PlanIndicatorStatus::Unknown => None,
    }
}

/// Indicators a reviewer wants called out whenever they are above zero, limit
/// or no limit: things the plan takes away, or runs with, that are bad news on
/// their own. Suffix-matched for the per-class family (`classes.<class>.lost`).
const ALWAYS_NOTABLE_WHEN_POSITIVE: &[&str] = &[
    ".lost",
    "impact.dependents.untouched",
    "supply_chain.providers.exploited",
    "supply_chain.providers.critical",
    "supply_chain.providers.blocked",
    "supply_chain.modules.blocked",
];

/// Is this indicator worth a line in a push's verdict? Over a limit, or under a
/// limit but not measurable (which is not a pass), or one of the few that
/// matter whenever they are non-zero. The headline numbers that are simply fine
/// are left to `plan show`, which lists every indicator.
pub fn is_notable(i: &PlanIndicator) -> bool {
    let limited = i.mode.is_some() && i.mode != Some(PlanIndicatorMode::Info);
    match i.status {
        Some(PlanIndicatorStatus::Block | PlanIndicatorStatus::Warn) => true,
        Some(PlanIndicatorStatus::Unknown) => limited,
        _ => {
            i.value.is_some_and(|v| v > 0.0)
                && i.key.as_deref().is_some_and(|k| {
                    ALWAYS_NOTABLE_WHEN_POSITIVE.iter().any(|n| {
                        if n.starts_with('.') {
                            k.starts_with("classes.") && k.ends_with(n)
                        } else {
                            k == *n
                        }
                    })
                })
        }
    }
}

/// One indicator as a line: `✕ Share of the workspace destroyed or replaced: 60% (limit 25%)`.
pub fn indicator_line(i: &PlanIndicator, style: Style) -> String {
    let mark = match indicator_gate(i) {
        Some(g) => g.mark(style),
        None => neutral_mark(style),
    };
    let label = i.label.as_deref().or(i.key.as_deref()).unwrap_or(DASH);
    let unit = i.unit.as_deref();
    let value = match i.value {
        Some(v) => with_unit(v, unit),
        None => "not measurable for this plan".to_string(),
    };
    let limit = match (i.limit, i.mode) {
        (Some(l), Some(PlanIndicatorMode::Warn)) => {
            format!(" (limit {}, warns only)", with_unit(l, unit))
        }
        (Some(l), _) => format!(" (limit {})", with_unit(l, unit)),
        (None, _) => String::new(),
    };
    format!("{mark} {label}: {value}{limit}")
}

// ── plan push ───────────────────────────────────────────────────────────────

/// The three-level verdict of a push: BLOCKED if the gate refused, WARN if it
/// passed with warnings, PASS otherwise. A missing verdict reads as PASS only
/// because the server always sends one; the caller treats `blocked` as the
/// authority for the exit code.
pub fn push_gate(r: &PlanPushResult) -> Gate {
    match r.verdict.as_ref() {
        Some(v) if v.blocked == Some(true) => Gate::Blocked,
        Some(v) if !v.warnings.is_empty() => Gate::Warn,
        _ => Gate::Pass,
    }
}

/// What the verdict lines need to know about the push that is not in the response.
#[derive(Debug, Clone, Copy)]
pub struct PushContext<'a> {
    pub workspace: &'a str,
    /// `--ref` as it was passed. `None` asks for the plan to gate the workspace -
    /// which the server may not grant; see [`Standing`].
    pub requested_ref: Option<&'a str>,
    /// Was the binary sent? A render-only plan can raise a block, never clear one.
    pub has_binary: bool,
}

/// Where a pushed plan stands, as the SERVER stored it. Not the same as what was
/// asked for: a CI token on a ref its trust entry does not allow to push state
/// may not push the gating plan either, so a push with no `--ref` from such a
/// pipeline is stored under a derived ref (`ci:<its ref>`) and gates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing<'a> {
    /// Stored with no ref: this is the plan that decides whether the workspace may be applied.
    Gating,
    /// Stored under the ref that was asked for: a verdict only.
    Ref(&'a str),
    /// No ref was asked for, and the server stored the plan under one anyway
    /// (the ref, when it said which).
    Demoted(Option<&'a str>),
}

/// Read a pushed plan's standing off the response. `gates_workspace` is the
/// authority; "was `--ref` passed" is only the fallback for a server from before
/// the field existed.
pub fn push_standing<'a>(r: &'a PlanPushResult, requested_ref: Option<&'a str>) -> Standing<'a> {
    let stored = r.plan_ref.as_deref();
    let gates = r
        .gates_workspace
        .unwrap_or(stored.is_none() && requested_ref.is_none());
    match (gates, requested_ref) {
        (true, _) => Standing::Gating,
        (false, Some(asked)) => Standing::Ref(stored.unwrap_or(asked)),
        (false, None) => Standing::Demoted(stored),
    }
}

/// The lines under `==> Verdict`: the verdict, each reason and each warning on
/// its own line, the risk score, the indicators worth noticing, and what the
/// verdict means for the workspace. `detail` is the version's review detail when
/// it could be fetched; without it the risk and indicator lines are simply absent.
pub fn push_verdict_lines(
    r: &PlanPushResult,
    detail: Option<&GetPlanVersionResponse>,
    ctx: PushContext<'_>,
    style: Style,
) -> Vec<String> {
    let gate = push_gate(r);
    let verdict = r.verdict.as_ref();
    let mut out = Vec::new();

    let mut head = gate.render(style);
    if gate == Gate::Blocked {
        let sources: Vec<String> = verdict
            .map(|v| v.sources.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        if !sources.is_empty() {
            head.push_str(&format!(" by {}", sources.join(" + ")));
        }
    }
    out.push(head);

    if let Some(v) = verdict {
        for reason in &v.reasons {
            out.push(format!("  - {reason}"));
        }
        if !v.warnings.is_empty() {
            // Under a block the warnings are a second list; under WARN they are
            // the verdict's own detail and need no heading.
            if gate == Gate::Blocked {
                out.push(format!(
                    "{} {} (not what blocked it)",
                    Gate::Warn.mark(style),
                    plural(v.warnings.len() as i64, "warning", "warnings")
                ));
            }
            for w in &v.warnings {
                out.push(format!("  - {w}"));
            }
        }
    }

    // The risk score is in the push response itself. The detail is only its
    // fallback, for a server from before the response carried it.
    let score = r
        .risk_score
        .or(detail.and_then(|d| d.risk.as_ref()).and_then(|k| k.score));
    let level = r.risk_level.map(|l| l.to_string()).or(detail
        .and_then(|d| d.risk.as_ref())
        .and_then(|k| k.level.map(|l| l.to_string())));
    if let Some(score) = score {
        out.push(format!(
            "risk {score} of 100 · {}",
            level.as_deref().unwrap_or(DASH)
        ));
    }

    // The indicators are not in the push response: they come from the version's
    // detail, when it could be read back.
    if let Some(d) = detail {
        for i in d.indicators.iter().flatten().filter(|i| is_notable(i)) {
            out.push(format!("  {}", indicator_line(i, style)));
        }
        if let Some(note) = d.run.as_ref().and_then(run_caveat) {
            out.push(note);
        }
    }

    // Can this plan be believed as the workspace's gating plan? Only when the
    // server has said how it was run: with its binary, not stale, not errored,
    // not -refresh-only. Unknown is not yes.
    let believable = ctx.has_binary
        && r.stale != Some(true)
        && detail
            .and_then(|d| d.run.as_ref())
            .is_some_and(|run| run.has_binary == Some(true) && run_caveat(run).is_none());
    out.extend(push_consequence(
        gate,
        push_standing(r, ctx.requested_ref),
        ctx.workspace,
        r.version_id.as_deref(),
        believable,
    ));
    out
}

/// A plan run in a way that cannot stand in for a real one.
fn run_caveat(run: &PlanRun) -> Option<String> {
    if run.errored == Some(true) {
        return Some(
            "the plan run errored part-way: it cannot be applied, lift a block or satisfy \"require a passing plan\""
                .into(),
        );
    }
    if run.mode.map(|m| m.to_string()).as_deref() == Some("refresh_only") {
        return Some(
            "a -refresh-only plan changes no infrastructure: it cannot lift a block or satisfy \"require a passing plan\""
                .into(),
        );
    }
    None
}

/// What the verdict does to the workspace. This is the sentence that used to
/// say "the next apply's LOCK is refused until an admin approves", which stopped
/// being true: a passing plan lifts a block too, and a plan with a ref never
/// raised one. Which case applies is the server's answer ([`Standing`]), not a
/// guess from the flags.
fn push_consequence(
    gate: Gate,
    standing: Standing<'_>,
    workspace: &str,
    version_id: Option<&str>,
    believable: bool,
) -> Vec<String> {
    let id = version_id.unwrap_or("<version-id>");
    let blocked = gate == Gate::Blocked;
    match standing {
        Standing::Gating if blocked => vec![
            format!(
                "apply is refused for workspace `{workspace}` (plan still works) until a passing plan is pushed or this plan is approved"
            ),
            format!("to approve: terramantle plan approve {workspace} {id} --reason \"…\""),
        ],
        // A render-only push was told what it cannot do before it was sent, and a
        // stale one is warned about above the verdict: neither is repeated here.
        Standing::Gating if believable => vec![format!(
            "this is now the plan that gates workspace `{workspace}`; any block an earlier plan raised is lifted"
        )],
        Standing::Gating => Vec::new(),
        Standing::Ref(r) if blocked => vec![
            format!(
                "this plan has a ref ({r}), so it does not block workspace `{workspace}`: applies there are unaffected"
            ),
            "its binary cannot be pulled until this version is approved".to_string(),
        ],
        Standing::Ref(r) => vec![format!(
            "pushed with ref {r}: a verdict only - it neither blocks nor unblocks the workspace"
        )],
        // Asked to be the gating plan, and was not allowed to be.
        Standing::Demoted(stored) => {
            let mut out = vec![
                format!(
                    "this plan was judged, but it is NOT the plan that gates workspace `{workspace}`"
                ),
                match stored {
                    Some(r) => format!(
                        "it was stored under ref {r}: this pipeline's ref is not one the workspace's trust entry allows to push state, so it may not push the gating plan either"
                    ),
                    None => "the server stored it under a ref: this pipeline's ref is not one the workspace's trust entry allows to push state, so it may not push the gating plan either"
                        .to_string(),
                },
                "it neither blocks nor unblocks the workspace; push from an allowed ref to gate it, or pass --ref to push a verdict-only plan deliberately"
                    .to_string(),
            ];
            if blocked {
                out.push("its binary cannot be pulled until this version is approved".to_string());
            }
            out
        }
    }
}

// ── policy results ──────────────────────────────────────────────────────────

/// Can a failure of this row block? The server says so in `enforcing`; a row
/// from before it did is enforcing only if it is one of the two gate aggregates
/// (no `policy_id`, and not an `-advisory` row).
fn is_enforcing(r: &PlanPolicyResult) -> bool {
    r.enforcing.unwrap_or_else(|| {
        r.policy_id.is_none()
            && !r
                .policy_name
                .as_deref()
                .is_some_and(|n| n.ends_with("-advisory"))
    })
}

/// One policy row's level. A failed ADVISORY row is WARN, never BLOCKED.
pub fn policy_gate(r: &PlanPolicyResult) -> Gate {
    match (r.passed.unwrap_or(false), is_enforcing(r)) {
        (true, _) => Gate::Pass,
        (false, true) => Gate::Blocked,
        (false, false) => Gate::Warn,
    }
}

/// The version's overall level, from its policy rows. `None` with no rows.
pub fn overall_gate(results: &[PlanPolicyResult]) -> Option<Gate> {
    results.iter().map(policy_gate).max_by_key(|g| match g {
        Gate::Pass => 0,
        Gate::Warn => 1,
        Gate::Blocked => 2,
    })
}

// ── plan show ───────────────────────────────────────────────────────────────

fn row(label: &str, value: impl AsRef<str>) -> String {
    format!("{label:<11}{}", value.as_ref())
}

fn continuation(value: impl AsRef<str>) -> String {
    row("", format!("  {}", value.as_ref()))
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn opt_i64(v: Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| DASH.into())
}

fn verdict_row(
    gate: Option<Gate>,
    approvals: Option<&ListPlanVersionApprovalsResponse>,
    style: Style,
) -> String {
    let Some(gate) = gate else {
        return row("verdict", DASH);
    };
    let mut text = gate.render(style);
    if gate == Gate::Blocked {
        if let Some(a) = approvals {
            let have = a.approvals.len() as i64;
            let need = a.required.unwrap_or(1);
            if a.approved == Some(true) {
                text.push_str(&format!(" · approved ({have} of {need})"));
            } else {
                text.push_str(&format!(" · not approved ({have} of {need})"));
            }
        }
    }
    row("verdict", text)
}

fn run_row(run: &PlanRun) -> String {
    if run.has_binary != Some(true) {
        return row(
            "run",
            "render only - how the plan was run is not known (no binary was pushed)",
        );
    }
    let mut parts = vec![match run.mode.map(|m| m.to_string()).as_deref() {
        Some("normal") => "normal plan".to_string(),
        Some("destroy") => "-destroy plan".to_string(),
        Some("refresh_only") => "-refresh-only plan".to_string(),
        Some(other) => other.to_string(),
        None => "mode not known".to_string(),
    }];
    if run.targeted == Some(true) {
        parts.push("targeted (-target/-exclude: a partial view)".into());
    }
    if run.errored == Some(true) {
        parts.push("ERRORED part-way (cannot be applied)".into());
    }
    parts.push("binary stored".into());
    row("run", parts.join(" · "))
}

/// The state a plan was made from, in two tenses. `pushed` is how it related to
/// the workspace's state WHEN IT WAS PUSHED (it does not change afterwards);
/// `stale_now` is whether it can still be applied, which does.
fn state_rows(
    v: &PlanVersionSummary,
    pushed: Option<&GetPlanVersionResponseState>,
    stale_now: Option<bool>,
) -> Vec<String> {
    // The detail's own state block when there is one; the summary otherwise.
    let source = pushed
        .and_then(|p| p.lineage_source.map(|s| s.to_string()))
        .or(v.lineage_source.map(|s| s.to_string()));
    let source = match source.as_deref() {
        Some("tfplan") => "read from the plan",
        Some("header") => "claimed by the pusher",
        _ => "not stated",
    };
    let mut text = format!(
        "serial {} · lineage {} ({source})",
        opt_i64(pushed.and_then(|p| p.prior_serial).or(v.prior_serial)),
        v.prior_lineage.as_deref().unwrap_or(DASH),
    );
    if let Some(n) = pushed.and_then(|p| p.managed_resources) {
        text.push_str(&format!(
            " · {} in it",
            plural(n, "managed resource", "managed resources")
        ));
    }
    let mut out = vec![row("state", text)];
    match pushed.and_then(|p| p.stale) {
        Some(true) => out.push(continuation(
            "when pushed: already stale (kept for the record, or a render-only plan the workspace had moved past)",
        )),
        Some(false) => out.push(continuation("when pushed: current")),
        None => {}
    }
    match stale_now {
        Some(true) => out.push(continuation(
            "now: STALE - the workspace's state has moved on, so it can no longer be applied",
        )),
        Some(false) => out.push(continuation("now: current - it can still be applied")),
        None => {}
    }
    out
}

fn provenance_rows(p: &PlanProvenance) -> Vec<String> {
    let source = p.source.map(|s| s.to_string());
    if p.verified != Some(true) {
        let what = match source.as_deref() {
            Some("machine") => "a machine token",
            Some("human") => "a human login",
            Some("ci") => "CI",
            _ => "an unknown source",
        };
        return vec![row(
            "produced",
            format!("by {what} · unverified (only a CI OIDC token proves where a plan was made)"),
        )];
    }
    let mut head = vec!["by CI · verified".to_string()];
    let provider = p.provider.map(|x| x.to_string());
    match (provider.as_deref(), p.repository.as_deref()) {
        (Some(pr), Some(repo)) => head.push(format!("{pr} {repo}")),
        (None, Some(repo)) => head.push(repo.to_string()),
        (Some(pr), None) => head.push(pr.to_string()),
        (None, None) => {}
    }
    if let Some(r) = p.ref_.as_deref() {
        let protected = match p.ref_protected {
            Some(true) => " (protected)",
            Some(false) => " (unprotected)",
            None => "",
        };
        match p.sha.as_deref() {
            Some(sha) => head.push(format!("{r}{protected} @ {}", short_sha(sha))),
            None => head.push(format!("{r}{protected}")),
        }
    }
    let mut out = vec![row("produced", head.join(" · "))];
    let mut tail = Vec::new();
    if let Some(w) = p.workflow_ref.as_deref().or(p.workflow.as_deref()) {
        tail.push(format!("workflow {w}"));
    }
    if let Some(a) = p.actor.as_deref() {
        tail.push(format!("triggered by {a}"));
    }
    if let Some(e) = p.event.as_deref() {
        tail.push(format!("on {e}"));
    }
    if let Some(id) = p.run_id.as_deref() {
        tail.push(format!("run {id}"));
    }
    if let Some(env) = p.environment.as_deref() {
        tail.push(format!("environment {env}"));
    }
    if let (Some(h), Some(b)) = (p.head_ref.as_deref(), p.base_ref.as_deref()) {
        tail.push(format!("pull request {h} -> {b}"));
    }
    if !tail.is_empty() {
        out.push(continuation(tail.join(" · ")));
    }
    out
}

fn impact_rows(impact: &PlanImpact) -> Vec<String> {
    let mut out = Vec::new();
    match (impact.graph, impact.dependents.as_ref()) {
        (Some(PlanImpactGraph::Unavailable), _) => out.push(row(
            "impact",
            "dependents not known: the plan removes something and the render carried no prior state to read dependencies from",
        )),
        (_, Some(d)) => match d.count {
            Some(0) => out.push(row(
                "impact",
                "nothing that survives this plan depends on what it removes",
            )),
            Some(n) => {
                let untouched = match d.untouched {
                    Some(u) if u > 0 => format!(
                        " - {u} of them the plan does not change at all, so they are not in its change list"
                    ),
                    _ => String::new(),
                };
                out.push(row(
                    "impact",
                    format!(
                        "{} that survive this plan depend on something it destroys or replaces{untouched}",
                        plural(n, "resource", "resources")
                    ),
                ));
                if !d.sample.is_empty() {
                    out.push(continuation(sample(&d.sample, Some(n))));
                }
            }
            None => out.push(row("impact", "dependents not known")),
        },
        (_, None) => {}
    }
    if let Some(c) = impact.consumers.as_ref() {
        let n = c.count.unwrap_or(c.workspaces.len() as i64);
        let label = if out.is_empty() { "impact" } else { "" };
        if n == 0 {
            out.push(row(label, "no other workspace reads this state"));
        } else {
            out.push(row(
                label,
                format!(
                    "{} this state as terraform_remote_state: {}",
                    if n == 1 {
                        "1 other workspace reads".to_string()
                    } else {
                        format!("{n} other workspaces read")
                    },
                    sample(&c.workspaces, Some(n))
                ),
            ));
        }
    }
    out
}

/// A provider worth naming: blocked, known-exploited, critical, malware, or not
/// positively scanned. `None` for one that is simply fine.
fn provider_flags(p: &PlanSupplyChainProvidersListItem) -> Option<(Gate, Vec<String>)> {
    let mut flags = Vec::new();
    let mut gate = Gate::Warn;
    if p.blocked == Some(true) {
        flags.push("blocked by this org".to_string());
        gate = Gate::Blocked;
    }
    if p.malware == Some(true) {
        flags.push("malware".to_string());
        gate = Gate::Blocked;
    }
    if let Some(k) = p.kev.filter(|k| *k > 0) {
        flags.push(format!("{k} known-exploited"));
    }
    if let Some(c) = p.critical.filter(|c| *c > 0) {
        flags.push(format!("{c} critical"));
    }
    // The server's own definition of unverified: anything not positively scanned.
    if p.scanned != Some(true) {
        flags.push(
            if p.version.is_none() {
                "unverified (version not known)"
            } else {
                "unverified (not scanned)"
            }
            .to_string(),
        );
    }
    (!flags.is_empty()).then_some((gate, flags))
}

fn module_flags(m: &PlanSupplyChainModulesListItem) -> Option<(Gate, Vec<String>)> {
    let mut flags = Vec::new();
    let mut gate = Gate::Warn;
    if m.blocked == Some(true) {
        flags.push("blocked by this org".to_string());
        gate = Gate::Blocked;
    }
    if let Some(c) = m.critical.filter(|c| *c > 0) {
        flags.push(format!("{c} critical"));
    }
    // The server's definition again: an INTERNAL module whose version is unknown
    // or is not a published one. An external module is not ours to verify.
    if m.origin == Some(PlanSupplyChainModulesListItemOrigin::Terramantle)
        && (m.version.is_none() || m.status.as_deref() != Some("published"))
    {
        flags.push(match (m.version.is_some(), m.status.as_deref()) {
            (false, _) => "unverified (version not known)".to_string(),
            (true, Some(s)) => format!("unverified (status {s})"),
            (true, None) => "unverified (not a published version)".to_string(),
        });
    }
    (!flags.is_empty()).then_some((gate, flags))
}

fn n(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

fn supply_rows(sc: &PlanSupplyChain, style: Style) -> Vec<String> {
    let mut out = vec![row(
        "supply",
        match sc.source {
            Some(PlanSupplyChainSource::Tfplan) => {
                "exact versions, from the lock file inside the plan binary"
            }
            Some(PlanSupplyChainSource::WorkspaceLockfile) => {
                "versions from the lock file last uploaded for the workspace (a claim, not this plan's)"
            }
            Some(PlanSupplyChainSource::Render) => {
                "no versions known (render only, no lock file): every provider is unverified"
            }
            None => "version source not stated",
        },
    )];
    if let Some(p) = sc.providers.as_ref() {
        out.push(row(
            "providers",
            format!(
                "{} total · {} scanned · {} unverified · {} critical · {} exploited · {} malware · {} blocked",
                n(p.total),
                n(p.scanned),
                n(p.unverified),
                n(p.critical),
                n(p.exploited),
                n(p.malware),
                n(p.blocked)
            ),
        ));
        for item in &p.list {
            if let Some((gate, flags)) = provider_flags(item) {
                out.push(continuation(format!(
                    "{} {} {} · {}",
                    gate.mark(style),
                    item.address.as_deref().unwrap_or(DASH),
                    item.version.as_deref().unwrap_or(DASH),
                    flags.join(" · ")
                )));
            }
        }
    }
    if let Some(m) = sc.modules.as_ref() {
        out.push(row(
            "modules",
            format!(
                "{} total · {} local · {} internal · {} external · {} unverified · {} critical · {} blocked",
                n(m.total),
                n(m.local),
                n(m.internal),
                n(m.external),
                n(m.unverified),
                n(m.critical),
                n(m.blocked)
            ),
        ));
        for item in &m.list {
            if let Some((gate, flags)) = module_flags(item) {
                out.push(continuation(format!(
                    "{} {} {} · {}",
                    gate.mark(style),
                    item.source.as_deref().unwrap_or(DASH),
                    item.version.as_deref().unwrap_or(DASH),
                    flags.join(" · ")
                )));
            }
        }
    }
    out
}

fn approvals_rows(a: &ListPlanVersionApprovalsResponse) -> Vec<String> {
    let have = a.approvals.len() as i64;
    let need = a.required.unwrap_or(1);
    let mut rule = Vec::new();
    if let Some(role) = a.approver_min_role {
        rule.push(format!("needs the {role} role or above"));
    }
    if a.allow_self_approval == Some(false) {
        rule.push("the person who pushed the plan may not approve it".to_string());
    }
    let state = if a.approved == Some(true) {
        "approved"
    } else {
        "not yet approved"
    };
    let mut head = format!("{have} of {need} · {state}");
    if !rule.is_empty() {
        head.push_str(&format!(" ({})", rule.join("; ")));
    }
    let mut out = vec![row("approvals", head)];
    if a.stale == Some(true) && have > 0 {
        out.push(continuation(
            "the plan is stale, so these approvals are history, not permission: a re-plan is a new version and needs its own",
        ));
    }
    for item in &a.approvals {
        // A reader is not shown who approved or why; the role and time still are.
        let who = item
            .approver_email
            .as_deref()
            .unwrap_or("(hidden from the reader role)");
        let mut line = format!(
            "{who} · {} · {}",
            item.approver_role.as_deref().unwrap_or(DASH),
            item.created_at
                .map(relative_time)
                .unwrap_or_else(|| DASH.into())
        );
        if let Some(reason) = item.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            line.push_str(&format!(" · \"{reason}\""));
        }
        out.push(continuation(line));
    }
    out
}

fn indicators_table(list: &[PlanIndicator], style: Style) -> String {
    let mut view = TableView::new(["status", "indicator", "value", "limit", "mode"]);
    for i in list {
        let status = match (i.status, indicator_gate(i)) {
            (_, Some(g)) => g.mark(style),
            (Some(PlanIndicatorStatus::Unknown), None) => {
                format!("{} unknown", neutral_mark(style))
            }
            (_, None) => neutral_mark(style),
        };
        let unit = i.unit.as_deref();
        view.row([
            status,
            i.label
                .clone()
                .or_else(|| i.key.clone())
                .unwrap_or_else(|| DASH.into()),
            i.value
                .map(|v| with_unit(v, unit))
                .unwrap_or_else(|| "not measurable".into()),
            i.limit
                .map(|l| with_unit(l, unit))
                .unwrap_or_else(|| DASH.into()),
            i.mode.map(|m| m.to_string()).unwrap_or_else(|| DASH.into()),
        ]);
    }
    view.render()
}

fn policy_table(results: &[PlanPolicyResult], style: Style) -> String {
    let mut view = TableView::new(["result", "policy", "kind", "detail"]);
    for r in results {
        view.row([
            policy_gate(r).render(style),
            r.policy_name.clone().unwrap_or_else(|| DASH.into()),
            format!(
                "{} · {}",
                r.source
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| DASH.into()),
                if is_enforcing(r) {
                    "enforcing"
                } else {
                    "advisory"
                }
            ),
            r.failure_msg.clone().unwrap_or_else(|| DASH.into()),
        ]);
    }
    view.render()
}

/// The whole of `plan show`, as text. `policy` and `approvals` are `None` when
/// that call failed; the page then says nothing about them rather than guessing.
pub fn render_show(
    detail: &GetPlanVersionResponse,
    policy: Option<&[PlanPolicyResult]>,
    approvals: Option<&ListPlanVersionApprovalsResponse>,
    style: Style,
) -> Option<String> {
    let v = detail.version.as_ref()?;
    let mut out: Vec<String> = Vec::new();

    out.push(format!("plan {}", v.id.as_deref().unwrap_or(DASH)));
    out.push(row(
        "ref",
        match v.plan_ref.as_deref() {
            Some(r) => format!("{r} (a verdict only: it never blocks or unblocks the workspace)"),
            None => "none (this is a plan that gates the workspace)".to_string(),
        },
    ));
    if policy.is_some() {
        out.push(verdict_row(policy.and_then(overall_gate), approvals, style));
    }
    out.push(row(
        "changes",
        format!(
            "+{} ~{} -{} ±{} · {} drift · {} sensitive",
            n(v.add_count),
            n(v.change_count),
            n(v.destroy_count),
            n(v.replace_count),
            n(v.drift_count),
            n(v.sensitive_change_count)
        ),
    ));

    // Risk: the score, then every point of it accounted for.
    let score = detail.risk.as_ref().and_then(|k| k.score).or(v.risk_score);
    let level = detail
        .risk
        .as_ref()
        .and_then(|k| k.level.map(|l| l.to_string()))
        .or(v.risk_level.map(|l| l.to_string()));
    if let Some(score) = score {
        out.push(row(
            "risk",
            format!("{score} of 100 · {}", level.as_deref().unwrap_or(DASH)),
        ));
        for f in detail.risk.iter().flat_map(|k| k.factors.iter()) {
            out.push(continuation(format!(
                "+{:<3} {}",
                n(f.points),
                f.detail.as_deref().or(f.key.as_deref()).unwrap_or(DASH)
            )));
        }
    }

    if let Some(run) = detail.run.as_ref() {
        out.push(run_row(run));
    }
    out.extend(state_rows(
        v,
        detail.state.as_ref(),
        approvals.and_then(|a| a.stale),
    ));
    if let Some(p) = v.provenance.as_ref() {
        out.extend(provenance_rows(p));
    }
    out.push(row(
        "pushed",
        format!(
            "{}{}",
            v.created_at
                .map(relative_time)
                .unwrap_or_else(|| DASH.into()),
            match (v.actor_name.as_deref(), v.actor_type.map(|t| t.to_string())) {
                (Some(name), Some(kind)) => format!(" by {name} ({kind})"),
                (Some(name), None) => format!(" by {name}"),
                _ => String::new(),
            }
        ),
    ));
    out.push(row(
        "binary",
        v.tfplan_sha256
            .as_deref()
            .map(|s| format!("sha256 {s}"))
            .unwrap_or_else(|| "none (render only: not pullable)".into()),
    ));
    out.push(row("attest", v.attestation_hash.as_deref().unwrap_or(DASH)));
    out.push(row(
        "scan",
        v.scan_status
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.into()),
    ));

    if let Some(impact) = detail.impact.as_ref() {
        out.extend(impact_rows(impact));
    }
    if let Some(sc) = detail.supply_chain.as_ref() {
        out.extend(supply_rows(sc, style));
    }
    if let Some(a) = approvals {
        // Worth showing when there is something to approve or someone has.
        let blocked = policy.and_then(overall_gate) == Some(Gate::Blocked);
        if blocked || !a.approvals.is_empty() {
            out.extend(approvals_rows(a));
        }
    }

    if let Some(list) = detail.indicators.as_deref().filter(|l| !l.is_empty()) {
        out.push(String::new());
        out.push(indicators_table(list, style));
    }
    if let Some(results) = policy.filter(|r| !r.is_empty()) {
        out.push(String::new());
        out.push(policy_table(results, style));
    }
    if !detail.findings.is_empty() {
        out.push(String::new());
        let mut view = TableView::new(["severity", "category", "title", "resource", "attr"]);
        for f in &detail.findings {
            view.row([
                f.severity
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| DASH.into()),
                f.category.clone().unwrap_or_else(|| DASH.into()),
                f.title.clone().unwrap_or_else(|| DASH.into()),
                f.resource.clone().unwrap_or_else(|| DASH.into()),
                f.attr.clone().unwrap_or_else(|| DASH.into()),
            ]);
        }
        out.push(view.render());
    }
    Some(format!("{}\n", out.join("\n")))
}

// ── plan approve ────────────────────────────────────────────────────────────

/// What an approval did. `approved: false` is not a failure: the approval was
/// recorded and the plan needs more of them.
pub fn approval_lines(r: &ApprovePlanVersionResponse, style: Style) -> Vec<String> {
    let have = n(r.approvals);
    let need = r.required.unwrap_or(1);
    if r.approved == Some(true) {
        let mut out = vec![format!(
            "{} approved · {have} of {need} approvals",
            Gate::Pass.mark(style)
        )];
        out.push(
            if r.cleared == Some(true) {
                "the block this plan raised on the workspace is lifted"
            } else {
                "this plan was not what was blocking the workspace, so nothing was lifted; its binary can now be pulled"
            }
            .to_string(),
        );
        out
    } else {
        let more = (need - have).max(1);
        vec![
            // Neither a pass nor a warning: an approval counted, more to come.
            format!(
                "{} recorded · {have} of {need} approvals",
                neutral_mark(style)
            ),
            format!(
                "{} must approve before this plan counts as approved; it stays blocked until then",
                if more == 1 {
                    "one more person".to_string()
                } else {
                    format!("{more} more people")
                }
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain() -> Style {
        Style::plain()
    }

    fn push_result(v: serde_json::Value) -> PlanPushResult {
        serde_json::from_value(v).expect("PlanPushResult")
    }

    fn detail(v: serde_json::Value) -> GetPlanVersionResponse {
        serde_json::from_value(v).expect("GetPlanVersionResponse")
    }

    fn base_push(verdict: serde_json::Value) -> PlanPushResult {
        push_result(json!({
            "ok": true, "version_id": "0f8fad5b-d9cb-469f-a165-70867728950e",
            "attestation_hash": "ab", "tfplan_sha256": "cd", "prior_serial": 4,
            "prior_lineage": "l", "lineage_source": "tfplan", "stale": false,
            "deduped": false, "verdict": verdict, "scan_status": "pending"
        }))
    }

    const MAIN: PushContext<'static> = PushContext {
        workspace: "prod-network",
        requested_ref: None,
        has_binary: true,
    };

    /// The version detail as the spec's own examples give it.
    fn full_detail() -> GetPlanVersionResponse {
        // A string, not `json!`: a document this deep exceeds the macro's
        // recursion limit.
        serde_json::from_str(
            r#"{
            "version": {
                "id": "0f8fad5b-d9cb-469f-a165-70867728950e", "plan_ref": null,
                "add_count": 1, "change_count": 2, "destroy_count": 3, "replace_count": 1,
                "noop_count": 9, "drift_count": 0, "sensitive_change_count": 1,
                "attestation_hash": "abab", "prior_serial": 4, "prior_lineage": "79007a8e",
                "terraform_version": "1.9.0", "scan_status": "published",
                "created_at": 1763287200000, "lineage_source": "tfplan",
                "render_kind": "full", "tfplan_sha256": "cdcd",
                "plan_mode": "normal", "plan_targeted": 0, "plan_errored": 0,
                "risk_score": 62, "risk_level": "high",
                "actor_type": "vcs", "actor_name": "acme/infra",
                "provenance": {
                    "verified": true, "source": "ci", "provider": "github",
                    "repository": "acme/infra", "ref": "main", "ref_type": "branch",
                    "sha": "5f2c1e0d9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d",
                    "workflow": "Deploy infrastructure",
                    "workflow_ref": "acme/infra/.github/workflows/deploy.yml@refs/heads/main",
                    "actor": "sam-ops", "run_id": "9912345678", "event": "push",
                    "environment": "production", "ref_protected": true,
                    "head_ref": null, "base_ref": null
                }
            },
            "findings": [
                { "id": "f1", "severity": "high", "title": "Stateful resource destroyed",
                  "category": "stateful_destroy", "resource": "aws_db_instance.main", "attr": null }
            ],
            "risk": {
                "score": 62, "level": "high",
                "factors": [
                    { "key": "stateful_lost", "points": 40, "detail": "2 stateful resources destroyed or replaced" },
                    { "key": "destructive_share", "points": 22, "detail": "60% of the workspace destroyed or replaced" }
                ],
                "destructive": { "count": 4, "percent_of_state": 60, "destroy_only": false },
                "replace": { "count": 1 },
                "classes": { "stateful": { "created": 0, "updated": 0, "destroyed": 2, "replaced": 0 } },
                "drift": { "count": 0 }, "unknown_after_apply": { "count": 3 },
                "outputs": { "changed": 1, "removed": 0 }
            },
            "indicators": [
                { "key": "destructive.count", "label": "Destroyed or replaced", "value": 4, "unit": null, "limit": null, "mode": "info", "status": "ok" },
                { "key": "destructive.percent_of_state", "label": "Share of the workspace destroyed or replaced", "value": 60, "unit": "%", "limit": 25, "mode": "enforce", "status": "block" },
                { "key": "score", "label": "Risk score", "value": 62, "unit": null, "limit": 40, "mode": "warn", "status": "warn" },
                { "key": "classes.stateful.lost", "label": "Stateful data: destroyed or replaced", "value": 2, "unit": null, "limit": null, "mode": "info", "status": "ok" },
                { "key": "classes.iam.lost", "label": "IAM: destroyed or replaced", "value": 0, "unit": null, "limit": 5, "mode": "enforce", "status": "ok" },
                { "key": "drift.percent", "label": "Share drifted", "value": null, "unit": "%", "limit": 10, "mode": "enforce", "status": "unknown" }
            ],
            "run": { "mode": "normal", "targeted": false, "errored": false, "has_binary": true },
            "state": { "stale": false, "lineage_source": "tfplan", "prior_serial": 4, "managed_resources": 12 },
            "outputs": [ { "name": "vpc_id", "action": "update", "sensitive": false } ],
            "impact": {
                "graph": "ok",
                "dependents": { "count": 5, "untouched": 3, "sample": ["null_resource.c", "terraform_data.d[0]", "terraform_data.d[1]", "module.leaf.terraform_data.inner", "terraform_data.b"] },
                "consumers": { "count": 1, "workspaces": ["app-prod"] }
            },
            "supply_chain": {
                "source": "tfplan",
                "providers": {
                    "total": 3, "versioned": 3, "scanned": 2, "unverified": 1, "critical": 1,
                    "exploited": 1, "malware": 0, "blocked": 1,
                    "list": [
                        { "address": "registry.terraform.io/hashicorp/aws", "version": "5.1.0", "scanned": true, "critical": 2, "high": 4, "kev": 1, "malware": false, "blocked": true },
                        { "address": "registry.terraform.io/hashicorp/random", "version": "3.6.0", "scanned": true, "critical": 0, "high": 0, "kev": 0, "malware": false, "blocked": false },
                        { "address": "example.com/acme/custom", "version": "0.1.0", "scanned": null, "critical": 0, "high": 0, "kev": 0, "malware": false, "blocked": false }
                    ]
                },
                "modules": {
                    "total": 3, "local": 1, "internal": 1, "external": 1, "blocked": 0,
                    "unverified": 1, "critical": 0,
                    "list": [
                        { "source": "./modules/net", "version": null, "origin": "local", "blocked": null, "status": null, "critical": 0, "high": 0 },
                        { "source": "registry.terramantle.dev/acme/vpc/aws", "version": "1.2.0", "origin": "terramantle", "blocked": false, "status": "pending", "critical": 0, "high": 0 },
                        { "source": "terraform-aws-modules/s3-bucket/aws", "version": "4.0.0", "origin": "registry", "blocked": null, "status": null, "critical": 0, "high": 0 }
                    ]
                }
            }
        }"#,
        )
        .expect("GetPlanVersionResponse")
    }

    fn policy_rows() -> Vec<PlanPolicyResult> {
        serde_json::from_value(json!([
            { "id": "1", "policy_id": null, "policy_name": "builtin-plan-gate", "source": "builtin", "passed": false, "failure_msg": "60% of the workspace destroyed or replaced exceeds the limit of 25%", "evaluated_at": 1763287200000i64, "enforcing": true },
            { "id": "2", "policy_id": null, "policy_name": "custom-rego-gate", "source": "opa", "passed": true, "failure_msg": null, "evaluated_at": 1763287200000i64, "enforcing": true },
            { "id": "3", "policy_id": null, "policy_name": "builtin-plan-advisory", "source": "builtin", "passed": false, "failure_msg": "Risk score 62 is over the limit of 40", "evaluated_at": 1763287200000i64, "enforcing": false },
            { "id": "4", "policy_id": "p-9", "policy_name": "No public buckets", "source": "opa", "passed": false, "failure_msg": "bucket is public", "evaluated_at": 1763287200000i64, "enforcing": false }
        ]))
        .unwrap()
    }

    fn approvals(v: serde_json::Value) -> ListPlanVersionApprovalsResponse {
        serde_json::from_value(v).unwrap()
    }

    // ── push ────────────────────────────────────────────────────────────────

    #[test]
    fn a_passing_push_says_pass_and_nothing_alarming() {
        let r =
            base_push(json!({ "blocked": false, "reasons": [], "sources": [], "warnings": [] }));
        assert_eq!(push_gate(&r), Gate::Pass);
        assert_eq!(push_verdict_lines(&r, None, MAIN, plain()), vec!["PASS"]);
    }

    /// A response from before `warnings` existed has no such key: absent is empty.
    #[test]
    fn a_verdict_without_a_warnings_key_is_a_pass() {
        let r = base_push(json!({ "blocked": false, "reasons": [], "sources": [] }));
        assert_eq!(push_gate(&r), Gate::Pass);
    }

    /// Advisory policies and warn-mode limits: WARN, each on its own line, and
    /// not a single word that suggests a block.
    #[test]
    fn warnings_alone_are_warn_not_blocked() {
        let r = base_push(json!({
            "blocked": false, "reasons": [], "sources": [],
            "warnings": ["Risk score 62 is over the limit of 40", "No public buckets: bucket is public"]
        }));
        assert_eq!(push_gate(&r), Gate::Warn);
        let lines = push_verdict_lines(&r, None, MAIN, plain());
        assert_eq!(
            lines,
            vec![
                "WARN",
                "  - Risk score 62 is over the limit of 40",
                "  - No public buckets: bucket is public",
            ]
        );
        assert!(!lines.join("\n").to_lowercase().contains("block"));
        assert!(!lines.join("\n").contains("refused"));
    }

    #[test]
    fn a_blocked_push_lists_each_reason_then_each_warning() {
        let r = base_push(json!({
            "blocked": true,
            "reasons": ["3 destroys exceed max_destroys 1", "aws_db_instance is on the destroy deny-list"],
            "sources": ["builtin", "opa"],
            "warnings": ["Risk score 62 is over the limit of 40"]
        }));
        assert_eq!(push_gate(&r), Gate::Blocked);
        let lines = push_verdict_lines(&r, None, MAIN, plain());
        assert_eq!(lines[0], "BLOCKED by builtin + opa");
        assert_eq!(lines[1], "  - 3 destroys exceed max_destroys 1");
        assert_eq!(lines[2], "  - aws_db_instance is on the destroy deny-list");
        assert_eq!(lines[3], "WARN 1 warning (not what blocked it)");
        assert_eq!(lines[4], "  - Risk score 62 is over the limit of 40");
    }

    /// The sentence that was wrong in 0.10: approval is no longer the only way
    /// out, the LOCK is not the only thing refused, and plan still works.
    #[test]
    fn a_blocked_gating_plan_says_how_the_block_is_lifted() {
        let r = base_push(
            json!({ "blocked": true, "reasons": ["x"], "sources": ["builtin"], "warnings": [] }),
        );
        let text = push_verdict_lines(&r, None, MAIN, plain()).join("\n");
        assert!(text.contains(
            "apply is refused for workspace `prod-network` (plan still works) until a passing plan is pushed or this plan is approved"
        ));
        assert!(text.contains(
            "terramantle plan approve prod-network 0f8fad5b-d9cb-469f-a165-70867728950e --reason"
        ));
        assert!(!text.contains("LOCK is refused until an admin approves"));
    }

    /// The push response with the fields the server now sends on every push.
    fn pushed(blocked: bool, plan_ref: Option<&str>, gates_workspace: bool) -> PlanPushResult {
        push_result(json!({
            "ok": true, "version_id": "0f8fad5b-d9cb-469f-a165-70867728950e",
            "attestation_hash": "ab", "tfplan_sha256": "cd", "prior_serial": 4,
            "prior_lineage": "l", "lineage_source": "tfplan", "stale": false, "deduped": false,
            "verdict": { "blocked": blocked, "reasons": if blocked { vec!["x"] } else { vec![] },
                         "sources": if blocked { vec!["builtin"] } else { vec![] }, "warnings": [] },
            "scan_status": "pending",
            "plan_ref": plan_ref, "gates_workspace": gates_workspace,
            "risk_score": 62, "risk_level": "high"
        }))
    }

    /// `gates_workspace` decides the consequence text, not whether --ref was passed.
    #[test]
    fn standing_is_read_from_the_response_not_the_flag() {
        // The ordinary cases agree with the flag.
        assert_eq!(
            push_standing(&pushed(false, None, true), None),
            Standing::Gating
        );
        assert_eq!(
            push_standing(&pushed(false, Some("pr-42"), false), Some("pr-42")),
            Standing::Ref("pr-42")
        );
        // No --ref, and the server stored it under a derived ref: demoted.
        assert_eq!(
            push_standing(&pushed(false, Some("ci:refs/pull/42/merge"), false), None),
            Standing::Demoted(Some("ci:refs/pull/42/merge"))
        );
        // The stored ref is the one reported, even if it differs from the flag.
        assert_eq!(
            push_standing(&pushed(false, Some("pr-042"), false), Some("pr-42")),
            Standing::Ref("pr-042")
        );
        // A server from before the fields: fall back to what was asked for.
        let old = base_push(json!({ "blocked": false, "reasons": [], "sources": [] }));
        assert_eq!(push_standing(&old, None), Standing::Gating);
        assert_eq!(push_standing(&old, Some("pr-1")), Standing::Ref("pr-1"));
    }

    /// A CI pipeline on a ref that may not push state asked to push the gating
    /// plan. The plan is judged, and the output must not let anyone believe it
    /// gates the workspace.
    #[test]
    fn a_demoted_push_says_plainly_that_it_does_not_gate() {
        let d = full_detail();
        let r = pushed(false, Some("ci:refs/pull/42/merge"), false);
        let text = push_verdict_lines(&r, Some(&d), MAIN, plain()).join("\n");
        assert!(text.starts_with("PASS"));
        assert!(text.contains(
            "this plan was judged, but it is NOT the plan that gates workspace `prod-network`"
        ));
        assert!(text.contains("it was stored under ref ci:refs/pull/42/merge"));
        assert!(text.contains("not one the workspace's trust entry allows to push state"));
        assert!(text.contains("push from an allowed ref to gate it, or pass --ref"));
        // Everything that would be true of a gating plan is absent.
        assert!(!text.contains("this is now the plan that gates"));
        assert!(!text.contains("is lifted"));
        assert!(!text.contains("apply is refused"));

        // Blocked and demoted: still BLOCKED (the caller exits 3), still not a
        // block on the workspace, and its own binary is not pullable.
        let r = pushed(true, Some("ci:refs/pull/42/merge"), false);
        assert_eq!(push_gate(&r), Gate::Blocked);
        let text = push_verdict_lines(&r, Some(&d), MAIN, plain()).join("\n");
        assert!(text.starts_with("BLOCKED by builtin"));
        assert!(text.contains("it is NOT the plan that gates workspace"));
        assert!(text.contains("its binary cannot be pulled until this version is approved"));
        assert!(!text.contains("apply is refused"));
        assert!(!text.contains("terramantle plan approve prod-network"));
    }

    /// The risk line comes from the push response, so it is there even when the
    /// version detail could not be read back. Only the indicator lines need it.
    #[test]
    fn risk_is_printed_from_the_push_response_alone() {
        let r = pushed(false, None, true);
        let without_detail = push_verdict_lines(&r, None, MAIN, plain());
        assert_eq!(without_detail, vec!["PASS", "risk 62 of 100 · high"]);

        let with_detail = push_verdict_lines(&r, Some(&full_detail()), MAIN, plain()).join("\n");
        assert_eq!(with_detail.matches("risk 62 of 100 · high").count(), 1);
        assert!(with_detail.contains("Share of the workspace destroyed or replaced: 60%"));
        assert!(with_detail.contains("this is now the plan that gates workspace `prod-network`"));

        // The response wins over the detail if the two ever disagree.
        let mut odd = pushed(false, None, true);
        odd.risk_score = Some(10);
        odd.risk_level = None;
        let text = push_verdict_lines(&odd, Some(&full_detail()), MAIN, plain()).join("\n");
        assert!(text.contains("risk 10 of 100 · high"), "{text}");
    }

    /// `state` on the detail is how the plan stood WHEN PUSHED; "now" is the
    /// approvals endpoint's. Either can be absent, and a plan stored stale with
    /// --allow-stale says so.
    #[test]
    fn show_separates_state_at_push_from_state_now() {
        let mut d = full_detail();
        d.state = Some(
            serde_json::from_value(json!({ "stale": true, "lineage_source": "header", "prior_serial": 9, "managed_resources": null })).unwrap(),
        );
        let text = render_show(&d, None, None, plain()).unwrap();
        // The state block's own serial and source are the ones shown.
        assert!(text.contains("state      serial 9 · lineage 79007a8e (claimed by the pusher)"));
        assert!(!text.contains("managed resource"));
        assert!(text.contains("when pushed: already stale"));
        assert!(
            !text.contains("now:"),
            "nothing is known about now without the approvals read"
        );

        d.state = None;
        let text = render_show(&d, None, None, plain()).unwrap();
        assert!(text.contains("state      serial 4 · lineage 79007a8e (read from the plan)"));
        assert!(!text.contains("when pushed"));

        d.state = Some(
            serde_json::from_value(json!({ "stale": false, "lineage_source": "tfplan", "prior_serial": 4, "managed_resources": 1 })).unwrap(),
        );
        let text = render_show(&d, None, None, plain()).unwrap();
        assert!(text.contains("· 1 managed resource in it"));
    }

    #[test]
    fn a_blocked_plan_with_a_ref_does_not_block_the_workspace() {
        let r = base_push(
            json!({ "blocked": true, "reasons": ["x"], "sources": ["opa"], "warnings": [] }),
        );
        let ctx = PushContext {
            requested_ref: Some("pr-12"),
            ..MAIN
        };
        let text = push_verdict_lines(&r, None, ctx, plain()).join("\n");
        assert!(text.contains("has a ref (pr-12), so it does not block workspace `prod-network`"));
        assert!(text.contains("cannot be pulled until this version is approved"));
        assert!(!text.contains("apply is refused"));

        // And a passing one says it changes nothing either way.
        let ok =
            base_push(json!({ "blocked": false, "reasons": [], "sources": [], "warnings": [] }));
        let text = push_verdict_lines(&ok, None, ctx, plain()).join("\n");
        assert!(text.contains("neither blocks nor unblocks the workspace"));
    }

    /// "This plan now gates the workspace" is said only when the server has
    /// confirmed how the plan was run. Unknown, render-only, stale, errored and
    /// -refresh-only all stay silent: none of them lifts a block.
    #[test]
    fn a_pass_is_only_called_the_gating_plan_when_it_can_be_believed() {
        let pass =
            || base_push(json!({ "blocked": false, "reasons": [], "sources": [], "warnings": [] }));
        let lifted = "any block an earlier plan raised is lifted";
        let d = full_detail();

        let text = push_verdict_lines(&pass(), Some(&d), MAIN, plain()).join("\n");
        assert!(text.contains("this is now the plan that gates workspace `prod-network`"));
        assert!(text.contains(lifted));

        // No detail read back: how it was run is not known.
        assert!(!push_verdict_lines(&pass(), None, MAIN, plain())
            .join("\n")
            .contains(lifted));
        // Render only.
        let render_only = PushContext {
            has_binary: false,
            ..MAIN
        };
        assert!(!push_verdict_lines(&pass(), Some(&d), render_only, plain())
            .join("\n")
            .contains(lifted));
        // Stored stale (--allow-stale).
        let mut stale = pass();
        stale.stale = Some(true);
        assert!(!push_verdict_lines(&stale, Some(&d), MAIN, plain())
            .join("\n")
            .contains(lifted));
        // -refresh-only.
        let mut refresh = full_detail();
        refresh.run = Some(
            serde_json::from_value(json!({ "mode": "refresh_only", "targeted": false, "errored": false, "has_binary": true })).unwrap(),
        );
        assert!(!push_verdict_lines(&pass(), Some(&refresh), MAIN, plain())
            .join("\n")
            .contains(lifted));
        // A plan with a ref never moves the gate, however clean.
        let pr = PushContext {
            requested_ref: Some("pr-1"),
            ..MAIN
        };
        assert!(!push_verdict_lines(&pass(), Some(&d), pr, plain())
            .join("\n")
            .contains(lifted));
        // A blocked plan certainly does not.
        let blocked = base_push(
            json!({ "blocked": true, "reasons": ["x"], "sources": ["builtin"], "warnings": [] }),
        );
        assert!(!push_verdict_lines(&blocked, Some(&d), MAIN, plain())
            .join("\n")
            .contains(lifted));
    }

    #[test]
    fn risk_and_the_notable_indicators_follow_the_verdict() {
        let r = base_push(
            json!({ "blocked": true, "reasons": ["limit"], "sources": ["builtin"], "warnings": [] }),
        );
        let d = full_detail();
        let lines = push_verdict_lines(&r, Some(&d), MAIN, plain());
        let text = lines.join("\n");
        assert!(lines.contains(&"risk 62 of 100 · high".to_string()));
        assert!(
            text.contains("  BLOCK Share of the workspace destroyed or replaced: 60% (limit 25%)")
        );
        assert!(text.contains("  WARN Risk score: 62 (limit 40, warns only)"));
        // A class the plan touches is notable even with no limit on it.
        assert!(text.contains("  -- Stateful data: destroyed or replaced: 2"));
        // Under a limit but unmeasurable: shown, and not as a pass.
        assert!(text.contains("  -- Share drifted: not measurable for this plan (limit 10%)"));
        // Fine and unremarkable: left to `plan show`.
        assert!(!text.contains("Destroyed or replaced: 4"));
        assert!(!text.contains("IAM"));
    }

    #[test]
    fn notability_rules() {
        let ind = |v: serde_json::Value| -> PlanIndicator { serde_json::from_value(v).unwrap() };
        let over = ind(
            json!({ "key": "score", "label": "Risk score", "value": 62, "unit": null, "limit": 40, "mode": "warn", "status": "warn" }),
        );
        assert!(is_notable(&over));
        assert_eq!(indicator_gate(&over), Some(Gate::Warn));

        let fine = ind(
            json!({ "key": "score", "label": "Risk score", "value": 12, "unit": null, "limit": 40, "mode": "enforce", "status": "ok" }),
        );
        assert!(!is_notable(&fine));
        assert_eq!(indicator_gate(&fine), Some(Gate::Pass));

        // Shown for context only: no judgement at all.
        let info = ind(
            json!({ "key": "destructive.count", "label": "Destroyed", "value": 4, "unit": null, "limit": null, "mode": "info", "status": "ok" }),
        );
        assert!(!is_notable(&info));
        assert_eq!(indicator_gate(&info), None);

        let unknown_unlimited = ind(
            json!({ "key": "x", "label": "X", "value": null, "unit": "%", "limit": null, "mode": "info", "status": "unknown" }),
        );
        assert!(!is_notable(&unknown_unlimited));

        // With no limit at all, a handful still matter when they are non-zero -
        // and only then, and only those. Keys are the server's (planIndicators).
        let unlimited = |key: &str, value: i64| {
            ind(
                json!({ "key": key, "label": key, "value": value, "unit": null, "limit": null, "mode": "info", "status": "ok" }),
            )
        };
        for key in [
            "classes.stateful.lost",
            "classes.iam.lost",
            "impact.dependents.untouched",
            "supply_chain.providers.exploited",
            "supply_chain.providers.critical",
            "supply_chain.providers.blocked",
            "supply_chain.modules.blocked",
        ] {
            assert!(
                is_notable(&unlimited(key, 1)),
                "{key} > 0 should be notable"
            );
            assert!(!is_notable(&unlimited(key, 0)), "{key} = 0 should not be");
        }
        for key in [
            // The same class twice would only repeat the line above it.
            "classes.stateful.changed",
            "destructive.count",
            "replace.count",
            "impact.consumers.count",
            "supply_chain.modules.external",
            "supply_chain.providers.unverified",
            "outputs.lost",
        ] {
            assert!(
                !is_notable(&unlimited(key, 3)),
                "{key} should be left to plan show"
            );
        }

        let fractional = ind(
            json!({ "key": "x", "label": "X", "value": 12.5, "unit": "%", "limit": 10, "mode": "enforce", "status": "block" }),
        );
        assert_eq!(
            indicator_line(&fractional, plain()),
            "BLOCK X: 12.5% (limit 10%)"
        );
    }

    #[test]
    fn a_refresh_only_or_errored_plan_is_called_out() {
        let r =
            base_push(json!({ "blocked": false, "reasons": [], "sources": [], "warnings": [] }));
        let mut d = full_detail();
        d.run = Some(
            serde_json::from_value(json!({ "mode": "refresh_only", "targeted": false, "errored": false, "has_binary": true })).unwrap(),
        );
        assert!(push_verdict_lines(&r, Some(&d), MAIN, plain())
            .join("\n")
            .contains("-refresh-only plan changes no infrastructure"));
        d.run = Some(
            serde_json::from_value(
                json!({ "mode": "normal", "targeted": false, "errored": true, "has_binary": true }),
            )
            .unwrap(),
        );
        assert!(push_verdict_lines(&r, Some(&d), MAIN, plain())
            .join("\n")
            .contains("errored part-way"));
    }

    // ── policy ──────────────────────────────────────────────────────────────

    #[test]
    fn an_advisory_failure_is_warn_and_never_blocked() {
        let rows = policy_rows();
        assert_eq!(policy_gate(&rows[0]), Gate::Blocked);
        assert_eq!(policy_gate(&rows[1]), Gate::Pass);
        assert_eq!(policy_gate(&rows[2]), Gate::Warn);
        assert_eq!(policy_gate(&rows[3]), Gate::Warn);
        assert_eq!(overall_gate(&rows), Some(Gate::Blocked));
        assert_eq!(overall_gate(&rows[1..]), Some(Gate::Warn));
        assert_eq!(overall_gate(&rows[1..2]), Some(Gate::Pass));
        assert_eq!(overall_gate(&[]), None);
    }

    /// A row from before the server sent `enforcing`: only the two gate
    /// aggregates can block.
    #[test]
    fn enforcing_is_inferred_for_rows_that_predate_the_field() {
        let rows: Vec<PlanPolicyResult> = serde_json::from_value(json!([
            { "id": "1", "policy_id": null, "policy_name": "builtin-plan-gate", "source": "builtin", "passed": false, "evaluated_at": 1 },
            { "id": "2", "policy_id": null, "policy_name": "custom-rego-advisory", "source": "opa", "passed": false, "evaluated_at": 1 },
            { "id": "3", "policy_id": "p", "policy_name": "Some policy", "source": "opa", "passed": false, "evaluated_at": 1 }
        ]))
        .unwrap();
        assert_eq!(policy_gate(&rows[0]), Gate::Blocked);
        assert_eq!(policy_gate(&rows[1]), Gate::Warn);
        assert_eq!(policy_gate(&rows[2]), Gate::Warn);
    }

    // ── show ────────────────────────────────────────────────────────────────

    #[test]
    fn show_renders_every_block_the_server_sends() {
        let d = full_detail();
        let rows = policy_rows();
        let a = approvals(json!({
            "approved": false, "required": 2, "approver_min_role": "admin",
            "allow_self_approval": false, "stale": false,
            "approvals": [
                { "approver_email": "sam@example.com", "approver_role": "admin",
                  "reason": "Reviewed the two replaced instances; expected.", "created_at": 1763287200000i64 }
            ]
        }));
        let text = render_show(&d, Some(&rows), Some(&a), plain()).unwrap();

        assert!(text.starts_with("plan 0f8fad5b-d9cb-469f-a165-70867728950e\n"));
        assert!(text.contains("ref        none (this is a plan that gates the workspace)"));
        assert!(text.contains("verdict    BLOCKED · not approved (1 of 2)"));
        assert!(text.contains("changes    +1 ~2 -3 ±1 · 0 drift · 1 sensitive"));
        // Risk and what it is made of.
        assert!(text.contains("risk       62 of 100 · high"));
        assert!(text.contains("+40  2 stateful resources destroyed or replaced"));
        assert!(text.contains("+22  60% of the workspace destroyed or replaced"));
        // Run, state, provenance.
        assert!(text.contains("run        normal plan · binary stored"));
        // State, in two tenses: as it was at the push (the detail's `state`
        // block) and as it is now (the approvals endpoint's `stale`).
        assert!(text.contains(
            "state      serial 4 · lineage 79007a8e (read from the plan) · 12 managed resources in it"
        ));
        assert!(text.contains("when pushed: current"));
        assert!(text.contains("now: current - it can still be applied"));
        assert!(text.contains(
            "produced   by CI · verified · github acme/infra · main (protected) @ 5f2c1e0"
        ));
        assert!(text.contains(
            "workflow acme/infra/.github/workflows/deploy.yml@refs/heads/main · triggered by sam-ops · on push · run 9912345678 · environment production"
        ));
        // Impact.
        assert!(text.contains(
            "impact     5 resources that survive this plan depend on something it destroys or replaces - 3 of them the plan does not change at all"
        ));
        assert!(text.contains("null_resource.c, terraform_data.d[0]"));
        assert!(
            text.contains("1 other workspace reads this state as terraform_remote_state: app-prod")
        );
        // Supply chain: counts, then only the ones worth naming.
        assert!(
            text.contains("supply     exact versions, from the lock file inside the plan binary")
        );
        assert!(text.contains(
            "providers  3 total · 2 scanned · 1 unverified · 1 critical · 1 exploited · 0 malware · 1 blocked"
        ));
        assert!(text.contains(
            "BLOCK registry.terraform.io/hashicorp/aws 5.1.0 · blocked by this org · 1 known-exploited · 2 critical"
        ));
        assert!(text.contains("WARN example.com/acme/custom 0.1.0 · unverified (not scanned)"));
        assert!(
            !text.contains("hashicorp/random"),
            "a clean provider is not listed"
        );
        assert!(text.contains(
            "modules    3 total · 1 local · 1 internal · 1 external · 1 unverified · 0 critical · 0 blocked"
        ));
        assert!(text.contains(
            "WARN registry.terramantle.dev/acme/vpc/aws 1.2.0 · unverified (status pending)"
        ));
        assert!(
            !text.contains("s3-bucket"),
            "an external module is not ours to verify"
        );
        assert!(!text.contains("./modules/net"));
        // Approvals: how many, of how many, who, when, why.
        assert!(text.contains(
            "approvals  1 of 2 · not yet approved (needs the admin role or above; the person who pushed the plan may not approve it)"
        ));
        assert!(text.contains("sam@example.com · admin · "));
        assert!(text.contains("\"Reviewed the two replaced instances; expected.\""));
        // Indicators: value against limit, with a status.
        assert!(text.contains("STATUS"));
        assert!(text.contains("Share of the workspace destroyed or replaced"));
        assert!(text.contains("60%"));
        assert!(text.contains("25%"));
        assert!(text.contains("not measurable"));
        // Per-policy results: the advisory failures are WARN.
        let policy_section: Vec<&str> = text
            .lines()
            .filter(|l| l.contains("builtin-plan-advisory") || l.contains("No public buckets"))
            .collect();
        assert_eq!(policy_section.len(), 2);
        for l in &policy_section {
            assert!(l.trim_start().starts_with("WARN"), "{l}");
            assert!(l.contains("advisory"));
        }
        let gate_line = text
            .lines()
            .find(|l| l.contains("builtin-plan-gate"))
            .unwrap();
        assert!(gate_line.trim_start().starts_with("BLOCKED"));
        assert!(gate_line.contains("enforcing"));
        // Findings still there.
        assert!(text.contains("Stateful resource destroyed"));
    }

    /// A version from before the blocks existed: all null, and the page simply
    /// has fewer rows. Nothing is invented and nothing panics.
    #[test]
    fn show_copes_with_a_version_that_predates_every_block() {
        let d = detail(json!({
            "version": {
                "id": "v1", "plan_ref": "pr-7", "add_count": 0, "change_count": 0, "destroy_count": 0,
                "replace_count": 0, "noop_count": 1, "drift_count": 0, "sensitive_change_count": 0,
                "attestation_hash": "ab", "prior_serial": null, "prior_lineage": null,
                "scan_status": "pending", "created_at": 1700000000000i64,
                "lineage_source": "none", "render_kind": "full", "tfplan_sha256": null
            },
            "findings": [], "risk": null, "indicators": null, "run": null, "outputs": null,
            "impact": null, "supply_chain": null
        }));
        let text = render_show(&d, None, None, plain()).unwrap();
        assert!(text.contains("ref        pr-7 (a verdict only"));
        assert!(text.contains("binary     none (render only: not pullable)"));
        assert!(text.contains("state      serial — · lineage — (not stated)"));
        // Row labels, at the start of a line: none of these blocks was sent.
        for absent in [
            "risk",
            "verdict",
            "run",
            "impact",
            "supply",
            "providers",
            "modules",
            "approvals",
            "produced",
            "STATUS",
            "RESULT",
        ] {
            assert!(
                !text.lines().any(|l| l.starts_with(absent)),
                "unexpected `{absent}` row in:\n{text}"
            );
        }
    }

    #[test]
    fn show_says_when_approvals_are_only_history() {
        let d = full_detail();
        let rows = policy_rows();
        let a = approvals(json!({
            "approved": true, "required": 1, "approver_min_role": "owner",
            "allow_self_approval": true, "stale": true,
            "approvals": [ { "approver_email": null, "approver_role": "owner", "reason": null, "created_at": 1763287200000i64 } ]
        }));
        let text = render_show(&d, Some(&rows), Some(&a), plain()).unwrap();
        assert!(text.contains("verdict    BLOCKED · approved (1 of 1)"));
        // Current when pushed, stale now: the two are separate facts.
        assert!(text.contains("when pushed: current"));
        assert!(text.contains("now: STALE - the workspace's state has moved on"));
        assert!(text.contains("these approvals are history, not permission"));
        // A reader sees the role and the time, not the person.
        assert!(text.contains("(hidden from the reader role) · owner · "));
        assert!(text.contains("needs the owner role or above"));
    }

    #[test]
    fn show_explains_an_unverified_plan_and_a_render_only_run() {
        let mut d = full_detail();
        d.version.as_mut().unwrap().provenance = Some(
            serde_json::from_value(json!({ "verified": false, "source": "machine" })).unwrap(),
        );
        d.run = Some(
            serde_json::from_value(
                json!({ "mode": null, "targeted": null, "errored": null, "has_binary": false }),
            )
            .unwrap(),
        );
        d.impact = Some(
            serde_json::from_value(json!({
                "graph": "unavailable",
                "dependents": { "count": null, "untouched": null, "sample": [] },
                "consumers": { "count": 0, "workspaces": [] }
            }))
            .unwrap(),
        );
        let text = render_show(&d, None, None, plain()).unwrap();
        assert!(text.contains("produced   by a machine token · unverified"));
        assert!(text.contains("run        render only - how the plan was run is not known"));
        assert!(text.contains("impact     dependents not known"));
        assert!(text.contains("no other workspace reads this state"));
    }

    #[test]
    fn show_needs_a_version() {
        let d = detail(json!({ "findings": [] }));
        assert!(render_show(&d, None, None, plain()).is_none());
    }

    // ── approve ─────────────────────────────────────────────────────────────

    #[test]
    fn approval_outcomes_read_as_the_spec_examples_mean_them() {
        let resp = |v: serde_json::Value| -> ApprovePlanVersionResponse {
            serde_json::from_value(v).unwrap()
        };
        // The spec's `one_of_two` example: recorded, not yet enough.
        let one = approval_lines(
            &resp(
                json!({ "ok": true, "cleared": false, "approved": false, "approvals": 1, "required": 2 }),
            ),
            plain(),
        );
        assert_eq!(one[0], "-- recorded · 1 of 2 approvals");
        assert!(one[1].contains("one more person must approve"));
        assert!(one[1].contains("stays blocked"));

        // The spec's `done` example: the second approval lifts the block.
        let done = approval_lines(
            &resp(
                json!({ "ok": true, "cleared": true, "approved": true, "approvals": 2, "required": 2 }),
            ),
            plain(),
        );
        assert_eq!(done[0], "OK approved · 2 of 2 approvals");
        assert!(done[1].contains("is lifted"));

        // Approved, but this plan was not what blocked the workspace.
        let other = approval_lines(
            &resp(
                json!({ "ok": true, "cleared": false, "approved": true, "approvals": 1, "required": 1 }),
            ),
            plain(),
        );
        assert!(other[1].contains("nothing was lifted"));

        let three = approval_lines(
            &resp(
                json!({ "ok": true, "cleared": false, "approved": false, "approvals": 1, "required": 4 }),
            ),
            plain(),
        );
        assert!(three[1].contains("3 more people"));
    }

    #[test]
    fn samples_say_how_many_were_left_out() {
        let items: Vec<String> = (0..12).map(|i| format!("r{i}")).collect();
        assert!(sample(&items, Some(40)).ends_with("(+32 more)"));
        assert_eq!(sample(&items[..2], Some(2)), "r0, r1");
        assert_eq!(num(60.0), "60");
        assert_eq!(num(12.5), "12.5");
    }
}
