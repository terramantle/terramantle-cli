//! Plan commands — `plan push / pull / ls / show`.
//!
//! `push` is what a pipeline runs after `terraform plan -out=tfplan`: it renders the
//! plan with the user's own binary (`show -json` needs the initialised providers),
//! sends render + binary to `<backend address>/plan`, and prints the gate's verdict.
//! `pull` is the other half: the apply job downloads the reviewed binary, verifies
//! its sha-256 against the server's header, and hands it to `terraform apply`.
//!
//! Where org and workspace come from: the stack's backend configuration
//! ([`tm_scaffold::resolve_backend`]), because the plan URL is derived from the
//! state address exactly as terraform derives `lock_address`. That also means the
//! backend's origin must be the API this CLI is talking to; a mismatch is an error,
//! not a silent push to the wrong host.
//!
//! Pure pieces (default plan-file lookup, sha verification, render rows) are
//! unit-tested with no network and no process spawn (rubric 3).

use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tm_api::generated::models::{PlanPushResult, PlanVersionSummary};
use tm_api::{ApiError, PlanPushOptions, PlanSelector};
use tm_auth::AuthContext;
use tm_scaffold::{resolve_backend, BackendAddress};

use crate::auth;
use crate::cli::{Cli, PlanCommand, PlanPullArgs, PlanPushArgs};
use crate::commands::CmdResult;
use crate::discovery::{api_fail, client_and_org};
use crate::output::{self, relative_time, TableView, DASH};
use crate::tf;

/// Exit 3: the gate blocked the plan (§9 posture-gate exit).
const EXIT_GATE: i32 = 3;
/// Exit 6: no plan file / no backend config to read.
const EXIT_NOT_FOUND: i32 = 6;

/// Saved-plan filenames tried when none is given, in order.
const DEFAULT_PLAN_FILES: &[&str] = &["tfplan", "plan.tfplan"];
/// Where `pull` writes when `--out` is absent.
const DEFAULT_OUT: &str = "plan.tfplan";

// ── dispatch ────────────────────────────────────────────────────────────────

pub fn plan(command: &PlanCommand, cli: &Cli) -> CmdResult {
    match command {
        PlanCommand::Push(args) => push(cli, args),
        PlanCommand::Pull(args) => pull(cli, args),
        PlanCommand::Ls {
            workspace,
            limit,
            chdir,
        } => ls(cli, workspace.as_deref(), *limit, chdir.as_deref()),
        PlanCommand::Show {
            workspace,
            version_id,
        } => show(cli, workspace, version_id),
    }
}

// ── shared ──────────────────────────────────────────────────────────────────

fn working_dir(chdir: Option<&str>) -> PathBuf {
    chdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve the backend address and check its origin is the API we are authed to.
/// A pipeline pointed at a different registry must not push there with this token.
fn backend_for(
    cli: &Cli,
    dir: &Path,
    address: Option<&str>,
) -> Result<(BackendAddress, AuthContext), i32> {
    let backend = match resolve_backend(dir, address) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return Err(EXIT_NOT_FOUND);
        }
    };
    let ctx = match auth::auth_context(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Err(5);
        }
    };
    let api = ctx.api_url.trim_end_matches('/');
    if !same_origin(api, &backend.api_origin) {
        eprintln!(
            "error: the backend address points at {} but this CLI is configured for {api}; pass --api-url {} (or --address) so the plan goes where the state lives",
            backend.api_origin, backend.api_origin
        );
        return Err(2);
    }
    Ok((backend, ctx))
}

/// Origins compare case-insensitively on scheme and host; a trailing slash is noise.
pub fn same_origin(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// A steady-tick stderr spinner — TTY only, never under `-o json`.
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

fn step(msg: &str) {
    eprintln!("==> {msg}");
}

fn detail(msg: &str) {
    eprintln!("    {msg}");
}

fn is_json(cli: &Cli) -> bool {
    matches!(
        cli.global.output,
        Some(tm_config::OutputFormat::Json) | Some(tm_config::OutputFormat::Yaml)
    )
}

fn stderr_tty() -> bool {
    supports_color::on(supports_color::Stream::Stderr).is_some()
}

/// Lowercase sha-256 hex of `bytes` — what the server's `X-Terramantle-Tfplan-Sha256` carries.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ── plan push ───────────────────────────────────────────────────────────────

/// The `-o json` shape of a push.
#[derive(Debug, Serialize)]
struct PushOutput<'a> {
    org: &'a str,
    workspace: &'a str,
    tfplan: Option<String>,
    #[serde(flatten)]
    result: &'a PlanPushResult,
}

/// Pick the saved-plan file: the explicit argument, else the first default name
/// present in `dir`. Pure over an `exists` probe so it is testable without a disk.
pub fn locate_plan_file(
    dir: &Path,
    explicit: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(p) = explicit {
        let path = Path::new(p);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            dir.join(path)
        };
        return exists(&full).then_some(full);
    }
    DEFAULT_PLAN_FILES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| exists(p))
}

fn push(cli: &Cli, args: &PlanPushArgs) -> CmdResult {
    let json = is_json(cli);
    let tty = stderr_tty();
    let dir = working_dir(args.chdir.as_deref());

    // ==> Reading the backend
    let (backend, ctx) = match backend_for(cli, &dir, args.address.as_deref()) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if !json {
        step("Resolving backend");
        detail(&format!(
            "org {} · workspace {}",
            backend.org, backend.workspace
        ));
    }

    // ==> Reading the plan
    let tfplan_path = if args.render_only {
        None
    } else {
        match locate_plan_file(&dir, args.tfplan.as_deref(), |p| p.is_file()) {
            Some(p) => Some(p),
            None => {
                eprintln!(
                    "error: no saved plan found{}; run `terraform plan -out=tfplan` first, name the file, or pass --render-only with --render",
                    args.tfplan.as_deref().map(|p| format!(" at {p}")).unwrap_or_else(|| format!(" (looked for {} in {})", DEFAULT_PLAN_FILES.join(", "), dir.display()))
                );
                return Ok(EXIT_NOT_FOUND);
            }
        }
    };
    let tfplan_bytes = match &tfplan_path {
        Some(p) => Some(std::fs::read(p)?),
        None => None,
    };

    let render = match &args.render {
        Some(f) => std::fs::read_to_string(f)?,
        None => {
            let Some(plan_path) = &tfplan_path else {
                eprintln!("error: --render-only needs --render <show-json-file>");
                return Ok(2);
            };
            let Some(bin) = tf::detect_tf_bin(args.tf_bin.as_deref()) else {
                eprintln!("error: {}", tf::RenderError::NoBinary);
                return Ok(EXIT_NOT_FOUND);
            };
            if !json {
                step(&format!("Rendering plan ({bin} show -json)"));
            }
            match tf::render_plan(&bin, &dir, plan_path) {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("error: {e}");
                    return Ok(1);
                }
            }
        }
    };
    if !json {
        if let Some(b) = &tfplan_bytes {
            detail(&format!(
                "{} · {} bytes · render {} bytes",
                tfplan_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                b.len(),
                render.len()
            ));
        } else {
            detail(&format!("render only · {} bytes", render.len()));
        }
    }

    // ==> Authenticating + pushing
    let client = match auth::api_client(&ctx) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(5);
        }
    };
    if !json {
        step(&format!("Pushing ({})", auth::mode_label(&ctx)));
    }
    let spinner = spinner_for("waiting for the policy gate", tty, json);
    let opts = PlanPushOptions {
        plan_ref: args.plan_ref.as_deref(),
        prior_serial: None,
        prior_lineage: None,
        allow_stale: args.allow_stale,
    };
    let pushed = client.plan_push(
        &backend.org,
        &backend.workspace,
        &render,
        tfplan_bytes.as_deref(),
        &opts,
    );
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    let result = match pushed {
        Ok(r) => r,
        Err(e) => return Ok(push_fail(&e)),
    };

    let blocked = result
        .verdict
        .as_ref()
        .and_then(|v| v.blocked)
        .unwrap_or(false);
    if json {
        let out = PushOutput {
            org: &backend.org,
            workspace: &backend.workspace,
            tfplan: tfplan_path.as_ref().map(|p| p.display().to_string()),
            result: &result,
        };
        output::print_structured(&out, cli.global.output.unwrap_or_default())?;
    } else {
        detail(&format!(
            "version {} · {}{}",
            result.version_id.as_deref().unwrap_or(DASH),
            match result
                .lineage_source
                .as_ref()
                .map(|s| s.to_string())
                .as_deref()
            {
                Some("tfplan") => format!(
                    "prior serial {} (from the plan)",
                    result
                        .prior_serial
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| DASH.into())
                ),
                Some("header") => format!(
                    "prior serial {} (claimed)",
                    result
                        .prior_serial
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| DASH.into())
                ),
                _ => "no prior serial".to_string(),
            },
            if result.deduped == Some(true) {
                " · already stored"
            } else {
                ""
            },
        ));
        if result.stale == Some(true) {
            eprintln!("warning: this plan is stale - the workspace has moved past the serial it was made against");
        }
        step("Verdict");
        match result.verdict.as_ref() {
            Some(v) if v.blocked == Some(true) => {
                detail(&format!(
                    "✕ blocked by {}",
                    v.sources
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(" + ")
                ));
                for r in &v.reasons {
                    detail(&format!("  - {r}"));
                }
                detail("the next apply's LOCK is refused until an admin approves this exact plan in the portal");
            }
            _ => detail("✓ allowed"),
        }
        if let Some(h) = result.attestation_hash.as_deref() {
            detail(&format!("attestation {h}"));
        }
    }
    Ok(if blocked { EXIT_GATE } else { 0 })
}

fn push_fail(e: &ApiError) -> i32 {
    eprintln!("error: {e}");
    e.exit_code()
}

// ── plan pull ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct PullOutput {
    org: String,
    workspace: String,
    out: String,
    bytes: usize,
    plan_id: Option<String>,
    attestation_hash: Option<String>,
    tfplan_sha256: Option<String>,
    prior_serial: Option<i64>,
    prior_lineage: Option<String>,
    stale: bool,
}

/// Verify a pulled body against the server's sha header. A missing header is a
/// failure too: the whole point of the pull is to apply exactly the reviewed bytes.
pub fn verify_sha(bytes: &[u8], expected: Option<&str>) -> Result<String, String> {
    let actual = sha256_hex(bytes);
    match expected {
        None => Err(
            "the server sent no X-Terramantle-Tfplan-Sha256 header; refusing to trust the download"
                .into(),
        ),
        Some(e) if e.eq_ignore_ascii_case(&actual) => Ok(actual),
        Some(e) => Err(format!(
            "sha-256 mismatch: server says {e}, downloaded bytes hash to {actual}"
        )),
    }
}

fn pull(cli: &Cli, args: &PlanPullArgs) -> CmdResult {
    let json = is_json(cli);
    let tty = stderr_tty();
    let dir = working_dir(args.chdir.as_deref());
    let (backend, ctx) = match backend_for(cli, &dir, args.address.as_deref()) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let client = match auth::api_client(&ctx) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(5);
        }
    };
    let selector = match args.id.as_deref() {
        Some(id) => PlanSelector::Id(id),
        None => PlanSelector::Latest {
            plan_ref: args.plan_ref.as_deref(),
        },
    };
    if !json {
        step(&format!(
            "Pulling {} ({})",
            match selector {
                PlanSelector::Id(id) => format!("plan {id}"),
                PlanSelector::Latest { plan_ref: Some(r) } => format!("latest plan for ref {r}"),
                PlanSelector::Latest { plan_ref: None } => "latest mainline plan".into(),
            },
            auth::mode_label(&ctx)
        ));
        detail(&format!(
            "org {} · workspace {}",
            backend.org, backend.workspace
        ));
    }
    let spinner = spinner_for("downloading", tty, json);
    let pulled = client.plan_pull(&backend.org, &backend.workspace, selector, args.allow_stale);
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    let pulled = match pulled {
        Ok(p) => p,
        Err(e) => return Ok(push_fail(&e)),
    };

    let sha = match verify_sha(&pulled.bytes, pulled.tfplan_sha256.as_deref()) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    };
    let out_path = {
        let p = PathBuf::from(args.out.as_deref().unwrap_or(DEFAULT_OUT));
        if p.is_absolute() {
            p
        } else {
            dir.join(p)
        }
    };
    std::fs::write(&out_path, &pulled.bytes)?;

    if json {
        output::print_structured(
            &PullOutput {
                org: backend.org.clone(),
                workspace: backend.workspace.clone(),
                out: out_path.display().to_string(),
                bytes: pulled.bytes.len(),
                plan_id: pulled.plan_id.clone(),
                attestation_hash: pulled.attestation_hash.clone(),
                tfplan_sha256: Some(sha),
                prior_serial: pulled.prior_serial,
                prior_lineage: pulled.prior_lineage.clone(),
                stale: pulled.stale,
            },
            cli.global.output.unwrap_or_default(),
        )?;
    } else {
        detail(&format!(
            "✓ {} · {} bytes · sha256 verified",
            out_path.display(),
            pulled.bytes.len()
        ));
        detail(&format!(
            "plan {} · prior serial {}",
            pulled.plan_id.as_deref().unwrap_or(DASH),
            pulled
                .prior_serial
                .map(|s| s.to_string())
                .unwrap_or_else(|| DASH.into())
        ));
        if pulled.stale {
            eprintln!("warning: this plan is stale; terraform will refuse to apply it against the current state");
        }
        detail(&format!(
            "apply with: terraform apply {}",
            out_path.display()
        ));
    }
    Ok(0)
}

// ── plan ls / show ──────────────────────────────────────────────────────────

/// One rendered `plan ls` row (also the `-o json` shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanRow {
    pub id: String,
    pub plan_ref: Option<String>,
    pub add: i64,
    pub change: i64,
    pub destroy: i64,
    pub replace: i64,
    pub drift: i64,
    pub prior_serial: Option<i64>,
    /// A binary is stored and pullable.
    pub binary: bool,
    pub scan_status: Option<String>,
    pub created_at: Option<i64>,
}

pub fn plan_rows(versions: &[PlanVersionSummary]) -> Vec<PlanRow> {
    versions
        .iter()
        .map(|v| PlanRow {
            id: v.id.clone().unwrap_or_default(),
            plan_ref: v.plan_ref.clone(),
            add: v.add_count.unwrap_or(0),
            change: v.change_count.unwrap_or(0),
            destroy: v.destroy_count.unwrap_or(0),
            replace: v.replace_count.unwrap_or(0),
            drift: v.drift_count.unwrap_or(0),
            prior_serial: v.prior_serial,
            binary: v.tfplan_sha256.is_some(),
            scan_status: v.scan_status.as_ref().map(|s| s.to_string()),
            created_at: v.created_at,
        })
        .collect()
}

/// Render the `plan ls` table (pure).
pub fn render_ls(rows: &[PlanRow]) -> String {
    let mut view = TableView::new([
        "id",
        "ref",
        "+add",
        "~change",
        "-destroy",
        "±replace",
        "drift",
        "serial",
        "binary",
        "scan",
        "pushed",
    ]);
    for r in rows {
        view.row([
            r.id.get(..8).unwrap_or(&r.id).to_string(),
            r.plan_ref.clone().unwrap_or_else(|| "main".into()),
            r.add.to_string(),
            r.change.to_string(),
            r.destroy.to_string(),
            r.replace.to_string(),
            r.drift.to_string(),
            r.prior_serial
                .map(|s| s.to_string())
                .unwrap_or_else(|| DASH.into()),
            if r.binary { "yes".into() } else { DASH.into() },
            r.scan_status.clone().unwrap_or_else(|| DASH.into()),
            r.created_at
                .map(relative_time)
                .unwrap_or_else(|| DASH.into()),
        ]);
    }
    format!("{}\n", view.render())
}

fn ls(cli: &Cli, workspace: Option<&str>, limit: u64, chdir: Option<&str>) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let workspace = match workspace {
        Some(w) => w.to_string(),
        None => {
            match resolve_backend(&working_dir(chdir), None) {
                Ok(b) => b.workspace,
                Err(e) => {
                    eprintln!("error: no workspace given and none could be read from the backend config: {e}");
                    return Ok(EXIT_NOT_FOUND);
                }
            }
        }
    };
    let versions = match client.plan_versions(&org, &workspace, limit, 0) {
        Ok(v) => v,
        Err(e) => return Ok(api_fail(&e)),
    };
    let rows = plan_rows(&versions);
    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&rows, format)? {
        return Ok(0);
    }
    print!("{}", render_ls(&rows));
    Ok(0)
}

fn show(cli: &Cli, workspace: &str, version_id: &str) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let detail_resp = match client.plan_version(&org, workspace, version_id) {
        Ok(v) => v,
        Err(e) => return Ok(api_fail(&e)),
    };
    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&detail_resp, format)? {
        return Ok(0);
    }
    let Some(v) = detail_resp.version.as_ref() else {
        eprintln!("error: the server returned no version summary for {version_id}");
        return Ok(1);
    };
    println!("plan {}", v.id.as_deref().unwrap_or(version_id));
    println!("ref        {}", v.plan_ref.as_deref().unwrap_or("main"));
    println!(
        "changes    +{} ~{} -{} ±{} · {} drift · {} sensitive",
        v.add_count.unwrap_or(0),
        v.change_count.unwrap_or(0),
        v.destroy_count.unwrap_or(0),
        v.replace_count.unwrap_or(0),
        v.drift_count.unwrap_or(0),
        v.sensitive_change_count.unwrap_or(0)
    );
    println!(
        "prior      serial {} · lineage {} ({})",
        v.prior_serial
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.into()),
        v.prior_lineage.as_deref().unwrap_or(DASH),
        v.lineage_source
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.into())
    );
    println!(
        "binary     {}",
        v.tfplan_sha256
            .as_deref()
            .map(|s| format!("sha256 {s}"))
            .unwrap_or_else(|| "none (render only)".into())
    );
    println!(
        "attest     {}",
        v.attestation_hash.as_deref().unwrap_or(DASH)
    );
    println!(
        "scan       {}",
        v.scan_status
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.into())
    );
    if !detail_resp.findings.is_empty() {
        println!();
        let mut view = TableView::new(["severity", "category", "title", "resource", "attr"]);
        for f in &detail_resp.findings {
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
        println!("{}", view.render());
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_plan_file_prefers_explicit_then_defaults() {
        let dir = Path::new("/w");
        let exists = |p: &Path| p == Path::new("/w/plan.tfplan") || p == Path::new("/abs/x.tfplan");
        assert_eq!(
            locate_plan_file(dir, Some("/abs/x.tfplan"), exists),
            Some(PathBuf::from("/abs/x.tfplan"))
        );
        assert_eq!(locate_plan_file(dir, Some("missing"), exists), None);
        assert_eq!(
            locate_plan_file(dir, None, exists),
            Some(PathBuf::from("/w/plan.tfplan"))
        );
        assert_eq!(
            locate_plan_file(dir, None, |p: &Path| p == Path::new("/w/tfplan")),
            Some(PathBuf::from("/w/tfplan"))
        );
        assert_eq!(locate_plan_file(dir, None, |_: &Path| false), None);
    }

    #[test]
    fn sha_verification_requires_a_matching_header() {
        let bytes = b"hello";
        let hex = sha256_hex(bytes);
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(verify_sha(bytes, Some(&hex.to_uppercase())).unwrap(), hex);
        assert!(verify_sha(bytes, None).is_err());
        assert!(verify_sha(bytes, Some("deadbeef"))
            .unwrap_err()
            .contains("mismatch"));
    }

    #[test]
    fn origins_compare_loosely() {
        assert!(same_origin(
            "https://registry.terramantle.dev/",
            "https://REGISTRY.terramantle.dev"
        ));
        assert!(!same_origin(
            "https://registry.terramantle.dev",
            "https://registry.example.com"
        ));
    }

    #[test]
    fn ls_rows_and_table_render() {
        let v: PlanVersionSummary = serde_json::from_value(serde_json::json!({
            "id": "0f8fad5b-d9cb-469f-a165-70867728950e", "plan_ref": null, "add_count": 1, "change_count": 2, "destroy_count": 0,
            "replace_count": 1, "noop_count": 9, "drift_count": 0, "sensitive_change_count": 1, "attestation_hash": "ab",
            "prior_serial": 4, "prior_lineage": "l", "terraform_version": "1.9.0", "scan_status": "published",
            "created_at": 1700000000000i64, "lineage_source": "tfplan", "render_kind": "full", "tfplan_sha256": "cd"
        }))
        .unwrap();
        let rows = plan_rows(&[v]);
        assert!(rows[0].binary);
        assert_eq!(rows[0].prior_serial, Some(4));
        let table = render_ls(&rows);
        assert!(table.contains("0f8fad5b"));
        assert!(table.contains("main"));
        assert!(table.contains("yes"));
    }
}
