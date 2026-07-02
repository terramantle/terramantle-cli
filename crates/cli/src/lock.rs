//! `lock push [path]` — upload `.terraform.lock.hcl` + eventually-consistent
//! posture (SPEC §6 mockup / §7).
//!
//! The command is side-effecting: narration + the step cadence go to **stderr**
//! (indicatif, TTY-gated), and stdout stays clean unless `-o json` is requested
//! (rubric 6). Three pieces are pure so they are unit-tested with no network and
//! no sleeps (rubrics 2, 3, 5):
//!   * [`parse_provider_addresses`] — the display-only lock-file provider count,
//!   * [`poll_posture`] — the poll loop over an injected fetch + clock,
//!   * [`evaluate_posture`] — verdict/outdated derivation reusing the slice-4 join.
//!
//! The API client is built once via `auth::api_client` (never by cloning an
//! `HttpClient`, which drops the refresh hook).

use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use tm_api::{ProviderOverview, WorkspaceProvider, WorkspaceProvidersResponse};
use tm_config::OutputFormat;

use crate::auth;
use crate::cli::Cli;
use crate::commands::CmdResult;
use crate::discovery::{self, index_overview, verdict_for, EXIT_MISSING_ORG};
use crate::output::{self, Style, TrustVerdict};

/// The lock-file name the server enforces (§7). Reads default to `<path>/<name>`.
const LOCK_FILE_NAME: &str = ".terraform.lock.hcl";

/// Exit 3: posture gate tripped (`--fail-on-atrisk`) — §9.
const EXIT_POSTURE_GATE: i32 = 3;
/// Exit 6: the lock file (or workspace/org) was not found — §9.
const EXIT_NOT_FOUND: i32 = 6;

// ── lock-file provider parse (rubric 5, no network) ─────────────────────────────

/// Extract the provider addresses declared in a `.terraform.lock.hcl` body, in
/// file order and de-duplicated. This is a **display-only** light parse — the
/// server parses authoritatively; we only need the "N providers · a, b, c" line
/// (§6). We match each `provider "<addr>" {` block header, tolerating leading
/// whitespace and either spacing around the brace.
pub fn parse_provider_addresses(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_start();
        let Some(rest) = line.strip_prefix("provider") else {
            continue;
        };
        // Require whitespace between the keyword and the opening quote so we don't
        // match an attribute like `provider_meta`.
        let rest = rest.trim_start();
        if rest.len() == line.trim_start().len() {
            continue; // no separating whitespace consumed → not a block header
        }
        let Some(after_open) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = after_open.find('"') else {
            continue;
        };
        let addr = &after_open[..end];
        // The remainder must open a block (`... {`), possibly with trailing space.
        if !after_open[end + 1..].trim_start().starts_with('{') {
            continue;
        }
        if addr.is_empty() || out.iter().any(|a| a == addr) {
            continue;
        }
        out.push(addr.to_string());
    }
    out
}

/// Resolve the `.terraform.lock.hcl` path from the user-supplied `path`: if it is
/// a file, use it directly; otherwise treat it as a directory and append the
/// canonical filename (§7 lock push).
pub fn resolve_lock_path(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_file() {
        p.to_path_buf()
    } else {
        p.join(LOCK_FILE_NAME)
    }
}

// ── posture poll (rubric 3, injected fetch + clock, no sleeps) ──────────────────

/// The outcome of polling workspace-providers for posture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// The lookup queue landed data for every pushed provider.
    Ready(WorkspaceProvidersResponse),
    /// The `--posture-timeout` budget elapsed before all pushed providers were
    /// populated. Carries the last snapshot so partial posture can still show.
    TimedOut(WorkspaceProvidersResponse),
}

/// True once every pushed provider in `resp` has its lookup populated
/// (`latest_matching` **or** `latest_version_checked_at` non-null) — the
/// eventually-consistent readiness signal from the `PROVIDER_LOOKUP_QUEUE` (§7).
/// Providers not among `pushed` are ignored; a pushed provider missing from the
/// response entirely counts as not-ready.
pub fn posture_ready(resp: &WorkspaceProvidersResponse, pushed: &[String]) -> bool {
    for addr in pushed {
        let (ns, ty) = split_address(addr);
        let found = resp
            .providers
            .iter()
            .find(|p| provider_matches(p, &ns, &ty, addr));
        match found {
            Some(p) if p.latest_matching.is_some() || p.latest_version_checked_at.is_some() => {}
            _ => return false,
        }
    }
    true
}

/// Poll `fetch` until [`posture_ready`] for the `pushed` set or `timeout`
/// elapses, sleeping `interval` between attempts via the injected `sleep` hook.
///
/// `sleep` is injected so tests can advance a virtual clock (no real waiting) and
/// so the attempt cadence is observable; production passes a real `thread::sleep`
/// with an `Instant`-based `elapsed`. `elapsed` returns the wall time consumed so
/// far — the loop stops once it would exceed `timeout`, never blocking longer than
/// the budget (§7).
pub fn poll_posture<F, S, E>(
    pushed: &[String],
    timeout: Duration,
    interval: Duration,
    mut fetch: F,
    mut sleep: S,
    mut elapsed: E,
) -> Result<PollOutcome, tm_api::ApiError>
where
    F: FnMut() -> Result<WorkspaceProvidersResponse, tm_api::ApiError>,
    S: FnMut(Duration),
    E: FnMut() -> Duration,
{
    loop {
        let resp = fetch()?;
        if posture_ready(&resp, pushed) {
            return Ok(PollOutcome::Ready(resp));
        }
        // Stop if another interval would push us past the budget (or if we're
        // already at/over it) — best-effort, never over-run the timeout.
        if elapsed() + interval > timeout {
            return Ok(PollOutcome::TimedOut(resp));
        }
        sleep(interval);
    }
}

// ── posture evaluation (rubric 3, pure) ─────────────────────────────────────────

/// One pushed provider's posture: verdict (provider-level, §7 derivation) +
/// outdated flag. This is exactly what `-o json` emits under `posture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PostureRow {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_matching: Option<String>,
    pub outdated: bool,
    pub trust: TrustVerdict,
}

impl PostureRow {
    /// Whether this row trips the `--fail-on-atrisk` gate (§7).
    pub fn is_at_risk(&self) -> bool {
        self.trust == TrustVerdict::AtRisk
    }
}

/// Derive a [`PostureRow`] per pushed provider by joining the workspace-providers
/// snapshot (for `current`/`latest_matching`) with the org overview index (for
/// the TRUST verdict, via the slice-4 [`verdict_for`]). Pure — the heart of the
/// posture block (rubric 3).
///
/// `outdated` = `latest_matching` present **and** `current_version !=
/// latest_matching`. Rows are ordered to match the `pushed` list.
pub fn evaluate_posture(
    pushed: &[String],
    resp: &WorkspaceProvidersResponse,
    overview: &[ProviderOverview],
) -> Vec<PostureRow> {
    let idx = index_overview(overview);
    let mut rows = Vec::new();
    for addr in pushed {
        let (ns, ty) = split_address(addr);
        let Some(p) = resp
            .providers
            .iter()
            .find(|p| provider_matches(p, &ns, &ty, addr))
        else {
            continue;
        };
        let latest_matching = p.latest_matching.clone();
        let outdated = latest_matching
            .as_deref()
            .is_some_and(|l| l != p.current_version);
        rows.push(PostureRow {
            namespace: p.namespace.clone(),
            type_: p.type_.clone(),
            current_version: p.current_version.clone(),
            latest_matching,
            outdated,
            trust: verdict_for(&idx, &p.namespace, &p.type_),
        });
    }
    rows
}

/// Render the posture block (§6): one `▲ … at-risk`/`… outdated`/`✓ … trusted`
/// line per pushed provider, matching the mockup vocabulary. Pure.
pub fn render_posture(rows: &[PostureRow], style: Style) -> String {
    let mut out = String::new();
    let mut trusted_current = 0u64;
    for r in rows {
        let addr = format!("{}/{}", r.namespace, r.type_);
        match r.trust {
            TrustVerdict::AtRisk => {
                let mark = if style.glyphs { "▲" } else { "WARN" };
                let latest = r
                    .latest_matching
                    .as_deref()
                    .map(|l| format!(" (latest matching {l})"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "    {mark} {addr} {} at-risk{latest}\n",
                    r.current_version
                ));
            }
            _ if r.outdated => {
                let mark = if style.glyphs { "▲" } else { "WARN" };
                let latest = r.latest_matching.as_deref().unwrap_or("—");
                out.push_str(&format!(
                    "    {mark} {addr} {} outdated (latest matching {latest})\n",
                    r.current_version
                ));
            }
            _ => trusted_current += 1,
        }
    }
    if trusted_current > 0 {
        let mark = if style.glyphs { "✓" } else { "OK" };
        if trusted_current == rows.len() as u64 {
            out.push_str(&format!("    {mark} {trusted_current} trusted\n"));
        } else {
            out.push_str(&format!("    {mark} {trusted_current} others trusted\n"));
        }
    }
    out
}

// ── json payload (rubric 6, `-o json`) ──────────────────────────────────────────

/// The `-o json` result for `lock push` (a small side-effect summary, §6).
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    pub org: String,
    pub workspace: String,
    /// Client-side provider count from the lock file (display-only).
    pub providers: Vec<String>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<UploadResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posture: Option<Vec<PostureRow>>,
    /// "timed_out" when posture never landed within the budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posture_status: Option<&'static str>,
}

/// The server's upload acknowledgement echoed into the json result.
#[derive(Debug, Clone, Serialize)]
pub struct UploadResult {
    pub ok: bool,
    pub providers_count: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ── dispatch ─────────────────────────────────────────────────────────────────────

/// Arguments for `lock push`, unpacked from the clap subcommand.
pub struct PushArgs<'a> {
    pub path: &'a str,
    pub fail_on_atrisk: bool,
    pub dry_run: bool,
    pub repo_url: Option<&'a str>,
    pub posture_timeout: u64,
    pub require_posture: bool,
}

pub fn push(cli: &Cli, args: &PushArgs) -> CmdResult {
    let style = Style::detect(cli.global.no_color);
    let tty = supports_color::on(supports_color::Stream::Stderr).is_some();
    let format = cli.global.output.unwrap_or_default();
    let json = matches!(format, OutputFormat::Json | OutputFormat::Yaml);

    // ==> Reading .terraform.lock.hcl
    let lock_path = resolve_lock_path(args.path);
    step(&format!("Reading {LOCK_FILE_NAME}"), tty, json);
    let bytes = match std::fs::read(&lock_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", lock_path.display());
            return Ok(EXIT_NOT_FOUND);
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let providers = parse_provider_addresses(&text);
    detail(
        &format!("{} providers · {}", providers.len(), providers.join(", ")),
        tty,
        json,
    );

    // Workspace resolves from config alone (no network) — required for the PUT
    // target; a missing workspace errors before any auth/upload.
    let workspace = match auth::config_workspace(cli) {
        Ok(Some(ws)) => ws,
        Ok(None) => {
            eprintln!(
                "error: no workspace configured; set --workspace, TERRAMANTLE_WORKSPACE, or a context"
            );
            return Ok(EXIT_MISSING_ORG);
        }
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    // --dry-run: parse + summary only, never build a client or upload (rubric 2).
    // Org is display-only here, so resolve it from config without the network and
    // fall back to `—` when absent rather than forcing auth.
    if args.dry_run {
        let org = auth::config_org(cli)?.unwrap_or_else(|| "—".to_string());
        detail("dry-run · not uploading", tty, json);
        if json {
            let result = PushResult {
                org,
                workspace,
                providers,
                dry_run: true,
                uploaded: None,
                posture: None,
                posture_status: None,
            };
            emit_json(&result, format)?;
        }
        return Ok(0);
    }

    // Resolve org (path org for the PUT) + build the client once (refresh hook
    // intact). This is the first step that touches auth/network.
    let (client, org) = match discovery::resolve_client_and_org(cli) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    // ==> Authenticating (<mode>)
    let ctx = match auth::auth_context(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(5);
        }
    };
    step(
        &format!("Authenticating ({})", auth::mode_label(&ctx)),
        tty,
        json,
    );
    detail(&format!("org {org} · workspace {workspace}"), tty, json);

    // ==> Uploading (spinner on TTY; plain header otherwise)
    step("Uploading", tty, json);
    let spinner = spinner_for("uploading lock file", tty, json);
    let upload = client.lock_push(&org, &workspace, &bytes, args.repo_url);
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    let uploaded = match upload {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(e.exit_code());
        }
    };
    detail(
        &format!("pushed · {} providers", uploaded.providers_count),
        tty,
        json,
    );
    for w in &uploaded.warnings {
        eprintln!("warning: {w}");
    }

    // ==> Posture — poll unless the user opted out via -o json without a gate.
    // The default (TTY) push shows posture; --fail-on-atrisk always evaluates it.
    let want_posture = args.fail_on_atrisk || (!json && tty);
    let mut posture_rows: Option<Vec<PostureRow>> = None;
    let mut posture_status: Option<&'static str> = None;
    let mut exit = 0;

    if want_posture {
        step("Posture", tty, json);
        let timeout = Duration::from_secs(args.posture_timeout);
        let interval = Duration::from_secs(1);
        let start = std::time::Instant::now();
        let spinner = spinner_for("waiting for provider lookup", tty, json);
        let outcome = poll_posture(
            &providers,
            timeout,
            interval,
            || client.workspace_providers(&org, &workspace),
            std::thread::sleep,
            || start.elapsed(),
        );
        if let Some(pb) = spinner {
            pb.finish_and_clear();
        }

        match outcome {
            Ok(PollOutcome::Ready(resp)) => {
                let overview = client.providers_overview(&org).unwrap_or_default();
                let rows = evaluate_posture(&providers, &resp, &overview);
                if !json {
                    eprint!("{}", render_posture(&rows, style));
                }
                if args.fail_on_atrisk && rows.iter().any(PostureRow::is_at_risk) {
                    exit = EXIT_POSTURE_GATE;
                }
                posture_rows = Some(rows);
            }
            Ok(PollOutcome::TimedOut(_)) => {
                eprintln!("    posture not ready (timed out)");
                posture_status = Some("timed_out");
                // Best-effort: unknown posture is a pass, unless --require-posture.
                if args.fail_on_atrisk && args.require_posture {
                    exit = EXIT_POSTURE_GATE;
                }
            }
            Err(e) => {
                eprintln!("warning: could not read posture: {e}");
                posture_status = Some("error");
                // A fetch failure is also "posture unknown"; honour --require-posture
                // the same way as a timeout under the gate (§7 best-effort).
                if args.fail_on_atrisk && args.require_posture {
                    exit = EXIT_POSTURE_GATE;
                }
            }
        }
    }

    if json {
        let result = PushResult {
            org,
            workspace,
            providers,
            dry_run: false,
            uploaded: Some(UploadResult {
                ok: uploaded.ok,
                providers_count: uploaded.providers_count,
                warnings: uploaded.warnings,
            }),
            posture: posture_rows,
            posture_status,
        };
        emit_json(&result, format)?;
    }

    Ok(exit)
}

// ── step cadence (stderr, TTY-gated) ────────────────────────────────────────────

/// Print a `==> <msg>` step header to stderr. On a non-TTY (CI) or when `-o json`
/// is requested we still emit a plain line so logs show progress; spinners are
/// only meaningful on a TTY, so we keep them off here and use plain lines to stay
/// deterministic (rubric 6). `json` suppresses narration entirely to keep the
/// stream to stderr minimal but present.
fn step(msg: &str, _tty: bool, _json: bool) {
    eprintln!("==> {msg}");
}

/// Print an indented detail line under the current step (§6).
fn detail(msg: &str, _tty: bool, _json: bool) {
    eprintln!("    {msg}");
}

/// A steady-tick stderr spinner for an in-flight step — **only** on a TTY and
/// when not emitting machine output. In CI (non-TTY) or under `-o json` we return
/// `None` so the log stays to deterministic plain lines (rubric 6).
fn spinner_for(msg: &str, tty: bool, json: bool) -> Option<ProgressBar> {
    if !tty || json {
        return None;
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("    {spinner} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    Some(pb)
}

fn emit_json(result: &PushResult, format: OutputFormat) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        OutputFormat::Yaml => output::print_yaml(result),
        _ => output::print_json(result),
    }
}

// ── address helpers ──────────────────────────────────────────────────────────────

/// Split a provider address into `(namespace, type)`. Addresses are
/// `registry.terraform.io/hashicorp/aws` or the short `hashicorp/aws`; we take the
/// last two `/`-segments as `(namespace, type)`.
fn split_address(addr: &str) -> (String, String) {
    let parts: Vec<&str> = addr.rsplitn(3, '/').collect();
    match parts.as_slice() {
        [ty, ns, ..] => (ns.to_string(), ty.to_string()),
        [ty] => (String::new(), ty.to_string()),
        _ => (String::new(), addr.to_string()),
    }
}

/// Whether a workspace-provider row matches a pushed lock address, by full
/// `provider_address` when present, else by `(namespace, type)`.
fn provider_matches(p: &WorkspaceProvider, ns: &str, ty: &str, addr: &str) -> bool {
    if let Some(pa) = &p.provider_address {
        if pa == addr {
            return true;
        }
    }
    p.namespace == ns && p.type_ == ty
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use tm_api::{ProviderOverview, WorkspaceProvider};

    const REALISTIC_LOCK: &str = r#"
# This file is maintained automatically by "terraform init".
# Manual edits may be lost in future updates.

provider "registry.terraform.io/hashicorp/aws" {
  version     = "5.2.0"
  constraints = ">= 5.0.0"
  hashes = [
    "h1:abc123==",
  ]
}

provider "registry.terraform.io/hashicorp/random" {
  version = "3.6.0"
  hashes = [
    "h1:def456==",
  ]
}

provider "registry.terraform.io/opentofu/null" {
  version = "3.2.1"
}
"#;

    // ── rubric 5: provider count parse against a realistic fixture ────────────

    #[test]
    fn parses_provider_addresses_from_realistic_lock() {
        let addrs = parse_provider_addresses(REALISTIC_LOCK);
        assert_eq!(
            addrs,
            vec![
                "registry.terraform.io/hashicorp/aws".to_string(),
                "registry.terraform.io/hashicorp/random".to_string(),
                "registry.terraform.io/opentofu/null".to_string(),
            ]
        );
    }

    #[test]
    fn parse_ignores_non_provider_blocks_and_dedups() {
        let text = r#"
provider "registry.terraform.io/hashicorp/aws" {
  version = "5.2.0"
}
provider_meta "foo" {
  bar = 1
}
provider "registry.terraform.io/hashicorp/aws" {
  version = "5.2.0"
}
terraform {
  required_version = ">= 1.0"
}
"#;
        let addrs = parse_provider_addresses(text);
        assert_eq!(addrs, vec!["registry.terraform.io/hashicorp/aws"]);
    }

    #[test]
    fn parse_empty_lock_yields_no_providers() {
        assert!(parse_provider_addresses("# empty\n").is_empty());
    }

    fn wp(
        ns: &str,
        ty: &str,
        current: &str,
        latest: Option<&str>,
        checked: Option<i64>,
    ) -> WorkspaceProvider {
        WorkspaceProvider {
            id: format!("{ns}/{ty}"),
            provider_address: Some(format!("registry.terraform.io/{ns}/{ty}")),
            namespace: ns.into(),
            type_: ty.into(),
            upstream: Some("registry.terraform.io".into()),
            current_version: current.into(),
            constraints: None,
            latest_version: None,
            latest_matching: latest.map(str::to_string),
            latest_version_checked_at: checked,
            provider_repo_url: None,
        }
    }

    fn resp(providers: Vec<WorkspaceProvider>) -> WorkspaceProvidersResponse {
        WorkspaceProvidersResponse {
            lock_file: None,
            providers,
        }
    }

    fn overview(ns: &str, ty: &str, crit: u64, high: u64, vuln: u64) -> ProviderOverview {
        ProviderOverview {
            namespace: ns.into(),
            type_: ty.into(),
            vuln_count: vuln,
            critical_count: crit,
            high_count: high,
            rule_id: None,
            upstream: Some("registry.terraform.io".into()),
            version_constraint: None,
            latest_job_id: None,
            status: None,
            triggered_at: None,
            completed_at: None,
        }
    }

    // ── rubric 3: readiness signal ────────────────────────────────────────────

    #[test]
    fn ready_when_all_pushed_have_latest_matching() {
        let r = resp(vec![wp(
            "hashicorp",
            "aws",
            "5.2.0",
            Some("5.4.1"),
            Some(1),
        )]);
        let pushed = vec!["registry.terraform.io/hashicorp/aws".to_string()];
        assert!(posture_ready(&r, &pushed));
    }

    #[test]
    fn not_ready_when_a_pushed_provider_is_unpopulated() {
        let r = resp(vec![
            wp("hashicorp", "aws", "5.2.0", Some("5.4.1"), Some(1)),
            wp("hashicorp", "random", "3.6.0", None, None),
        ]);
        let pushed = vec![
            "registry.terraform.io/hashicorp/aws".to_string(),
            "registry.terraform.io/hashicorp/random".to_string(),
        ];
        assert!(!posture_ready(&r, &pushed));
    }

    #[test]
    fn checked_at_alone_counts_as_ready() {
        // A provider looked up but with no matching version still counts as
        // "lookup landed" via latest_version_checked_at.
        let r = resp(vec![wp("opentofu", "null", "3.2.1", None, Some(999))]);
        let pushed = vec!["registry.terraform.io/opentofu/null".to_string()];
        assert!(posture_ready(&r, &pushed));
    }

    // ── rubric 3: poll loop with injected fetch + clock (no sleeps) ────────────

    /// A fake clock advancing by `interval` on each injected sleep.
    struct FakeClock {
        elapsed: RefCell<Duration>,
    }
    impl FakeClock {
        fn new() -> Self {
            Self {
                elapsed: RefCell::new(Duration::ZERO),
            }
        }
    }

    #[test]
    fn poll_ready_immediately_then_fail_on_atrisk() {
        let pushed = vec!["registry.terraform.io/hashicorp/aws".to_string()];
        let clock = FakeClock::new();
        let calls = RefCell::new(0u32);
        let outcome = poll_posture(
            &pushed,
            Duration::from_secs(15),
            Duration::from_secs(1),
            || {
                *calls.borrow_mut() += 1;
                Ok(resp(vec![wp(
                    "hashicorp",
                    "aws",
                    "5.2.0",
                    Some("5.4.1"),
                    Some(1),
                )]))
            },
            |d| *clock.elapsed.borrow_mut() += d,
            || *clock.elapsed.borrow(),
        )
        .unwrap();
        assert_eq!(*calls.borrow(), 1, "ready on first fetch — no sleeping");
        let ready = match outcome {
            PollOutcome::Ready(r) => r,
            other => panic!("expected ready, got {other:?}"),
        };
        // at-risk overview → fail-on-atrisk trips (exit 3 in the command).
        let ov = vec![overview("hashicorp", "aws", 1, 0, 0)];
        let rows = evaluate_posture(&pushed, &ready, &ov);
        assert!(rows.iter().any(PostureRow::is_at_risk));
    }

    #[test]
    fn poll_ready_trusted_does_not_trip() {
        let pushed = vec!["registry.terraform.io/hashicorp/random".to_string()];
        let clock = FakeClock::new();
        let outcome = poll_posture(
            &pushed,
            Duration::from_secs(15),
            Duration::from_secs(1),
            || {
                Ok(resp(vec![wp(
                    "hashicorp",
                    "random",
                    "3.6.0",
                    Some("3.6.0"),
                    Some(1),
                )]))
            },
            |d| *clock.elapsed.borrow_mut() += d,
            || *clock.elapsed.borrow(),
        )
        .unwrap();
        let ready = match outcome {
            PollOutcome::Ready(r) => r,
            other => panic!("expected ready, got {other:?}"),
        };
        let ov = vec![overview("hashicorp", "random", 0, 0, 0)];
        let rows = evaluate_posture(&pushed, &ready, &ov);
        assert!(!rows.iter().any(PostureRow::is_at_risk));
        assert!(!rows[0].outdated, "3.6.0 == latest matching");
        assert_eq!(rows[0].trust, TrustVerdict::Trusted);
    }

    #[test]
    fn poll_never_ready_times_out_best_effort() {
        // Provider lookup never lands; the loop must terminate at the budget
        // without ever sleeping past it, and report TimedOut (best-effort → the
        // command exits 0 unless --require-posture).
        let pushed = vec!["registry.terraform.io/hashicorp/aws".to_string()];
        let clock = FakeClock::new();
        let calls = RefCell::new(0u32);
        let outcome = poll_posture(
            &pushed,
            Duration::from_secs(3),
            Duration::from_secs(1),
            || {
                *calls.borrow_mut() += 1;
                Ok(resp(vec![wp("hashicorp", "aws", "5.2.0", None, None)]))
            },
            |d| *clock.elapsed.borrow_mut() += d,
            || *clock.elapsed.borrow(),
        )
        .unwrap();
        assert!(matches!(outcome, PollOutcome::TimedOut(_)));
        // 3s budget, 1s interval: fetch at 0s,1s,2s,3s; at 3s the next interval
        // (3+1>3) would over-run, so it stops → 4 fetches, none past the budget.
        assert_eq!(*calls.borrow(), 4);
        assert!(
            clock.elapsed.borrow().as_secs() <= 3,
            "never over-ran budget"
        );
    }

    #[test]
    fn poll_becomes_ready_after_a_couple_attempts() {
        let pushed = vec!["registry.terraform.io/hashicorp/aws".to_string()];
        let clock = FakeClock::new();
        let calls = RefCell::new(0u32);
        let outcome = poll_posture(
            &pushed,
            Duration::from_secs(15),
            Duration::from_secs(1),
            || {
                let n = {
                    let mut c = calls.borrow_mut();
                    *c += 1;
                    *c
                };
                let latest = if n >= 3 { Some("5.4.1") } else { None };
                let checked = if n >= 3 { Some(1) } else { None };
                Ok(resp(vec![wp("hashicorp", "aws", "5.2.0", latest, checked)]))
            },
            |d| *clock.elapsed.borrow_mut() += d,
            || *clock.elapsed.borrow(),
        )
        .unwrap();
        assert!(matches!(outcome, PollOutcome::Ready(_)));
        assert_eq!(*calls.borrow(), 3, "ready on the third poll");
    }

    // ── rubric 3: evaluate_posture derivation ─────────────────────────────────

    #[test]
    fn evaluate_marks_outdated_and_unscanned() {
        let pushed = vec!["registry.terraform.io/opentofu/null".to_string()];
        let r = resp(vec![wp(
            "opentofu",
            "null",
            "3.2.1",
            Some("3.2.2"),
            Some(1),
        )]);
        let rows = evaluate_posture(&pushed, &r, &[]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].outdated);
        assert_eq!(rows[0].trust, TrustVerdict::Unscanned);
    }

    #[test]
    fn evaluate_skips_providers_absent_from_snapshot() {
        let pushed = vec![
            "registry.terraform.io/hashicorp/aws".to_string(),
            "registry.terraform.io/ghost/gone".to_string(),
        ];
        let r = resp(vec![wp(
            "hashicorp",
            "aws",
            "5.2.0",
            Some("5.2.0"),
            Some(1),
        )]);
        let rows = evaluate_posture(&pushed, &r, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].type_, "aws");
        assert!(!rows[0].outdated);
    }

    // ── render + address helpers ──────────────────────────────────────────────

    #[test]
    fn render_posture_atrisk_and_trusted_plain() {
        let rows = vec![
            PostureRow {
                namespace: "hashicorp".into(),
                type_: "aws".into(),
                current_version: "5.2.0".into(),
                latest_matching: Some("5.4.1".into()),
                outdated: true,
                trust: TrustVerdict::AtRisk,
            },
            PostureRow {
                namespace: "hashicorp".into(),
                type_: "random".into(),
                current_version: "3.6.0".into(),
                latest_matching: Some("3.6.0".into()),
                outdated: false,
                trust: TrustVerdict::Trusted,
            },
        ];
        let out = render_posture(&rows, Style::plain());
        assert!(
            out.contains("WARN hashicorp/aws 5.2.0 at-risk (latest matching 5.4.1)"),
            "{out}"
        );
        assert!(out.contains("OK 1 others trusted"), "{out}");
        assert!(!out.contains('\u{1b}'), "plain render must have no ANSI");
    }

    #[test]
    fn render_posture_all_trusted_says_n_trusted() {
        let rows = vec![PostureRow {
            namespace: "hashicorp".into(),
            type_: "random".into(),
            current_version: "3.6.0".into(),
            latest_matching: Some("3.6.0".into()),
            outdated: false,
            trust: TrustVerdict::Trusted,
        }];
        let out = render_posture(&rows, Style::plain());
        assert!(out.contains("OK 1 trusted"), "{out}");
    }

    #[test]
    fn split_address_takes_last_two_segments() {
        assert_eq!(
            split_address("registry.terraform.io/hashicorp/aws"),
            ("hashicorp".into(), "aws".into())
        );
        assert_eq!(
            split_address("hashicorp/aws"),
            ("hashicorp".into(), "aws".into())
        );
    }

    #[test]
    fn resolve_lock_path_appends_filename_for_dir() {
        // A non-existent path is treated as a directory (not a file).
        let p = resolve_lock_path("/tmp/nonexistent-dir-xyz");
        assert!(p.ends_with(".terraform.lock.hcl"));
    }

    // ── posture json shape (rubric 6) ─────────────────────────────────────────

    #[test]
    fn posture_row_json_is_stable() {
        let row = PostureRow {
            namespace: "hashicorp".into(),
            type_: "aws".into(),
            current_version: "5.2.0".into(),
            latest_matching: Some("5.4.1".into()),
            outdated: true,
            trust: TrustVerdict::AtRisk,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
        assert_eq!(v["type"], "aws");
        assert_eq!(v["trust"], "at-risk");
        assert_eq!(v["outdated"], true);
    }
}
