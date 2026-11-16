//! Plan commands — `plan push / pull / ls / show / approve`.
//!
//! `push` is what a pipeline runs after `terraform plan -out=tfplan`: it checks the
//! plan against the workspace's current state, renders it with the user's own
//! binary (`show -json` needs the initialised providers), sends render + binary to
//! `<backend address>/plan`, and prints the gate's verdict. `pull` is the other
//! half: the apply job downloads the binary, verifies its sha-256 against the
//! server's header and against the hash the plan job recorded, and hands it to
//! `terraform apply`. `show` and `approve` are the review in between.
//!
//! What a push means depends on `--ref`. Without one, the plan gates the
//! workspace: blocked, it refuses the next apply until a passing plan is pushed or
//! it is approved. With one (a pull request's plan) it gets a verdict and never
//! blocks or unblocks anything. The server has the last word on which it was: a
//! CI pipeline on a ref that may not push state may not push the gating plan
//! either, and the response's `gates_workspace` - not the flag - is what the
//! output reports.
//!
//! Where org and workspace come from: the stack's backend configuration
//! ([`tm_scaffold::resolve_backend`]), because the plan URL is derived from the
//! state address exactly as terraform derives `lock_address`. That also means the
//! backend's origin must be the API this CLI is talking to; a mismatch is an error,
//! not a silent push to the wrong host.
//!
//! The pure pieces live beside this file and are unit-tested with no network and
//! no process spawn (rubric 3): reading the plan zip ([`crate::tfplan_zip`]), the
//! pre-flight rules ([`crate::plan_preflight`]), refusal text
//! ([`crate::plan_refusal`]) and every rendering ([`crate::plan_view`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tm_api::generated::models::{
    GetPlanVersionResponse, ListPlanVersionApprovalsResponse, PlanIndicator, PlanPolicyResult,
    PlanPushResult, PlanRisk, PlanVersionSummary,
};
use tm_api::{ApiError, Client, PlanPushOptions, PlanSelector};
use tm_auth::jwt::{self, TokenType};
use tm_auth::mode::AuthMode;
use tm_auth::AuthContext;
use tm_scaffold::{resolve_backend, BackendAddress};

use crate::auth;
use crate::cli::{Cli, PlanCommand, PlanPullArgs, PlanPushArgs};
use crate::commands::CmdResult;
use crate::discovery::client_and_org;
use crate::output::{self, relative_time, Gate, Style, TableView, DASH};
use crate::plan_preflight::{self, Preflight, StatePointer};
use crate::plan_refusal::{explain, Explained, PlanOp};
use crate::plan_view::{self, PushContext, Standing};
use crate::tf;
use crate::tfplan_zip::{self, PlanPrior};

/// Exit 2: what was passed in is wrong (a file that is not a saved plan, a bad flag).
const EXIT_USAGE: i32 = 2;
/// Exit 3: the gate blocked the plan (§9 posture-gate exit).
const EXIT_GATE: i32 = 3;
/// Exit 5: no usable credentials, or an identity that may not do this.
const EXIT_AUTH: i32 = 5;
/// Exit 6: no plan file / no backend config to read.
const EXIT_NOT_FOUND: i32 = 6;
/// Exit 7: the plan does not fit the workspace's current state (§9 state conflict).
/// The pre-flight uses the code the server's `plan_stale` / `lineage_mismatch`
/// already map to, so a pipeline sees one exit whichever side noticed.
const EXIT_STATE_CONFLICT: i32 = 7;

/// Saved-plan filenames tried when none is given, in order.
const DEFAULT_PLAN_FILES: &[&str] = &["tfplan", "plan.tfplan"];
/// Where `pull` writes when `--out` is absent.
const DEFAULT_OUT: &str = "plan.tfplan";
/// The longest approval reason the server accepts.
const MAX_REASON_CHARS: usize = 500;

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
        PlanCommand::Approve {
            workspace,
            version_id,
            reason,
        } => approve(cli, workspace, version_id, reason),
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
            return Err(EXIT_AUTH);
        }
    };
    let api = ctx.api_url.trim_end_matches('/');
    if !same_origin(api, &backend.api_origin) {
        eprintln!(
            "error: the backend address points at {} but this CLI is configured for {api}; pass --api-url {} (or --address) so the plan goes where the state lives",
            backend.api_origin, backend.api_origin
        );
        return Err(EXIT_USAGE);
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

/// Report a refusal and return its exit code. The sentence goes to stderr for a
/// person; under `-o json` the same refusal, with its stable `error` code, goes
/// to stdout so a pipeline that parses the output can branch on it.
fn fail(cli: &Cli, x: &Explained) -> CmdResult {
    for line in x.lines() {
        eprintln!("{line}");
    }
    if is_json(cli) {
        output::print_structured(x, cli.global.output.unwrap_or_default())?;
    }
    Ok(x.exit_code)
}

/// Is the caller a CI or bot identity rather than a person? Decided from how the
/// CLI is authenticating; a raw `TERRAMANTLE_TOKEN` is read for its claims, since
/// `auth env` exports a human's token the same way a pipeline sets a bot's. An
/// unreadable token is assumed human so the server gets to decide.
fn machine_identity(ctx: &AuthContext) -> bool {
    match ctx.mode {
        AuthMode::GitHub | AuthMode::GitLab | AuthMode::ClientCredentials => true,
        AuthMode::Device => false,
        AuthMode::Raw => std::env::var("TERRAMANTLE_TOKEN")
            .ok()
            .and_then(|t| jwt::decode_claims(&t).ok())
            .map(|c| c.token_type() != TokenType::Human)
            .unwrap_or(false),
    }
}

/// The binary's name as the user would type it (`tofu`, `terraform`), for
/// instructions. Falls back to `terraform` when none could be detected.
fn tf_name(flag: Option<&str>) -> String {
    tf::detect_tf_bin(flag)
        .and_then(|b| {
            Path::new(&b)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "terraform".to_string())
}

// ── plan push ───────────────────────────────────────────────────────────────

/// The `-o json` shape of a push: the server's response, whole (so `plan_ref`,
/// `gates_workspace`, `risk_score` and `risk_level` are the server's), plus where
/// it went, the three-level verdict, and the full risk block and indicators when
/// the version's detail could be read back.
#[derive(Debug, Serialize)]
struct PushOutput<'a> {
    org: &'a str,
    workspace: &'a str,
    tfplan: Option<String>,
    /// `pass` | `warn` | `blocked`. `blocked` is the only one that exits non-zero.
    gate: Gate,
    #[serde(flatten)]
    result: &'a PlanPushResult,
    risk: Option<&'a PlanRisk>,
    indicators: Option<&'a [PlanIndicator]>,
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

/// What the pre-flight could learn about the workspace's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerRead {
    /// The newest state version.
    State(StatePointer),
    /// The workspace has no state, or does not exist yet (a first push creates it).
    NoState,
    /// It could not be read. The pre-flight is skipped and the server decides.
    Unknown(String),
}

/// Read the workspace's current state pointer: the newest state version.
fn current_pointer(client: &Client, org: &str, workspace: &str) -> PointerRead {
    match client.state_versions(org, workspace, 1, 0) {
        Ok(versions) => match versions.first() {
            Some(v) => PointerRead::State(StatePointer {
                serial: v.serial,
                lineage: v.lineage.clone().filter(|l| !l.is_empty()),
            }),
            None => PointerRead::NoState,
        },
        Err(e) if e.error_code() == Some("not_found") => PointerRead::NoState,
        Err(e) => PointerRead::Unknown(match (e.error_code(), e.status()) {
            (Some(code), _) => code.to_string(),
            (None, Some(status)) => format!("HTTP {status}"),
            (None, None) => e.to_string(),
        }),
    }
}

/// What to do with a pre-flight answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightAction {
    /// The plan is current: upload it.
    Proceed,
    /// Not current, but `--allow-stale` was given: say so and upload it for the record.
    ProceedForTheRecord(Vec<String>),
    /// Not current: refuse before uploading anything.
    Refuse(Explained),
}

/// Decide what a pre-flight answer means for the push (pure).
pub fn preflight_action(
    pre: &Preflight,
    allow_stale: bool,
    workspace: &str,
    tf: &str,
) -> PreflightAction {
    let (Some(code), Some(headline)) = (pre.code(), pre.headline()) else {
        return PreflightAction::Proceed;
    };
    let mut lines = pre.explain(workspace, tf);
    if allow_stale {
        let mut warning = vec![format!("warning: {headline} ({code})")];
        warning.extend(lines.into_iter().map(|l| format!("    {l}")));
        warning.push(
            "    pushing it anyway for the record (--allow-stale): it cannot be applied, and the server may still refuse it"
                .to_string(),
        );
        return PreflightAction::ProceedForTheRecord(warning);
    }
    lines.push(
        "Nothing was uploaded. To store this plan for the record anyway, add --allow-stale; it still cannot be applied."
            .to_string(),
    );
    PreflightAction::Refuse(Explained::local(code, headline, lines, EXIT_STATE_CONFLICT))
}

/// After an upload: the server must have stored exactly the bytes that were
/// sent. `sent` is the local hash (`None` for a render-only push, which has
/// nothing to compare); `returned` is the response's `tfplan_sha256`.
pub fn verify_stored_sha(sent: Option<&str>, returned: Option<&str>) -> Result<(), Explained> {
    let Some(sent) = sent else {
        return Ok(());
    };
    let problem = match returned {
        Some(r) if r.eq_ignore_ascii_case(sent) => return Ok(()),
        Some(r) => format!("The binary sent has sha256 {sent}; the server recorded {r}."),
        None => format!(
            "The binary sent has sha256 {sent}; the server recorded no binary for this version."
        ),
    };
    Err(Explained::local(
        "tfplan_sha_mismatch",
        "the server did not store the plan binary that was sent",
        vec![
            problem,
            "The stored plan is not the one on this machine, so do not apply or approve it. Push again; if it repeats, something between here and the server is altering the upload."
                .to_string(),
        ],
        1,
    ))
}

/// The warning for a render-only push: what the server cannot know, and what
/// such a plan can never do.
pub fn render_only_warning(has_claim: bool) -> Vec<String> {
    let mut out = Vec::new();
    if has_claim {
        out.push(
            "note: render-only push: the prior serial and lineage are this command's claim, not something the server can read from a plan binary"
                .to_string(),
        );
    } else {
        out.push(
            "warning: render-only push with no --prior-serial / --prior-lineage: the server cannot verify what state this plan was made from"
                .to_string(),
        );
    }
    out.push(
        "    a render-only plan can raise a block on the workspace, but can never lift one or satisfy \"require a passing plan\"; push the binary for that"
            .to_string(),
    );
    out
}

/// The one-line summary of what was stored.
fn stored_line(result: &PlanPushResult) -> String {
    let serial = result
        .prior_serial
        .map(|s| s.to_string())
        .unwrap_or_else(|| DASH.into());
    let prior = match result
        .lineage_source
        .as_ref()
        .map(|s| s.to_string())
        .as_deref()
    {
        Some("tfplan") => format!("prior serial {serial} (from the plan)"),
        Some("header") => format!("prior serial {serial} (claimed)"),
        _ => "no prior serial".to_string(),
    };
    format!(
        "version {} · {prior}{}",
        result.version_id.as_deref().unwrap_or(DASH),
        // Only a plan that had passed or been approved comes back deduped; a
        // re-push of a still-blocked plan is a new version, judged again.
        if result.deduped == Some(true) {
            " · the same version as before (this exact plan was already pushed)"
        } else {
            ""
        },
    )
}

/// An absolute form of `p`, so a path found relative to `--chdir` still names the
/// same file once the render runs INSIDE `--chdir`.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(p))
        .unwrap_or_else(|_| p.to_path_buf())
}

fn push(cli: &Cli, args: &PlanPushArgs) -> CmdResult {
    let json = is_json(cli);
    let tty = stderr_tty();
    let style = Style::detect_stderr(cli.global.no_color);
    let dir = working_dir(args.chdir.as_deref());

    // ==> Reading the backend
    let (backend, ctx) = match backend_for(cli, &dir, args.address.as_deref()) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if !json {
        step("Resolving backend");
        detail(&format!(
            "org {} · workspace {}{}",
            backend.org,
            backend.workspace,
            match args.plan_ref.as_deref() {
                Some(r) => format!(" · ref {r} (verdict only)"),
                None => " · no ref (to be the plan that gates the workspace)".to_string(),
            }
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

    // The state the plan was computed from, read out of the plan itself. A file
    // this cannot read is one the server would refuse too, after the upload.
    let prior: Option<PlanPrior> = match (&tfplan_path, &tfplan_bytes) {
        (Some(path), Some(bytes)) => match tfplan_zip::read_prior_state(bytes) {
            Ok(p) => Some(p),
            Err(e) => {
                return fail(
                    cli,
                    &Explained::local(
                        "invalid_tfplan",
                        format!("{} is not a saved plan: {e}", path.display()),
                        vec![
                        "Push the file written by `plan -out=…`, unmodified. Nothing was uploaded."
                            .to_string(),
                    ],
                        EXIT_USAGE,
                    ),
                )
            }
        },
        _ => None,
    };

    let render = match &args.render {
        Some(f) => std::fs::read_to_string(f)?,
        None => {
            let Some(plan_path) = &tfplan_path else {
                eprintln!("error: --render-only needs --render <show-json-file>");
                return Ok(EXIT_USAGE);
            };
            let Some(bin) = tf::detect_tf_bin(args.tf_bin.as_deref()) else {
                eprintln!("error: {}", tf::RenderError::NoBinary);
                return Ok(EXIT_NOT_FOUND);
            };
            if !json {
                step(&format!("Rendering plan ({bin} show -json)"));
            }
            match tf::render_plan(&bin, &dir, &absolute(plan_path)) {
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

    // ==> Authenticating
    let client = match auth::api_client(&ctx) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_AUTH);
        }
    };

    // ==> Pre-flight: is the plan made from the state the workspace holds now?
    if let Some(prior) = &prior {
        if !json {
            step("Checking the plan against the workspace's current state");
        }
        let tf = tf_name(args.tf_bin.as_deref());
        let pointer = current_pointer(&client, &backend.org, &backend.workspace);
        let current = match &pointer {
            PointerRead::State(p) => Some(Some(p)),
            PointerRead::NoState => Some(None),
            PointerRead::Unknown(why) => {
                // Not a reason to stop: the server makes the same check on arrival.
                eprintln!(
                    "warning: could not read the workspace's current state ({why}), so the plan was not pre-checked; the server will check it"
                );
                None
            }
        };
        if let Some(current) = current {
            let pre = plan_preflight::check(prior, current);
            match preflight_action(&pre, args.allow_stale, &backend.workspace, &tf) {
                PreflightAction::Proceed => {
                    if !json {
                        detail(&match current {
                            Some(p) => format!(
                                "current: made from serial {}, which is where the workspace is",
                                p.serial
                            ),
                            // No state on the server is never stale, whatever the
                            // plan was made from: a first apply, or a workspace whose
                            // state lives elsewhere.
                            None => {
                                "current: the workspace has no state yet, so there is nothing for a plan to be behind"
                                    .to_string()
                            }
                        });
                    }
                }
                PreflightAction::ProceedForTheRecord(lines) => {
                    for l in lines {
                        eprintln!("{l}");
                    }
                }
                PreflightAction::Refuse(x) => return fail(cli, &x),
            }
        }
    } else {
        for l in render_only_warning(args.prior_serial.is_some() || args.prior_lineage.is_some()) {
            eprintln!("{l}");
        }
    }

    // ==> Pushing
    if !json {
        step(&format!("Pushing ({})", auth::mode_label(&ctx)));
    }
    let spinner = spinner_for("waiting for the policy gate", tty, json);
    let opts = PlanPushOptions {
        plan_ref: args.plan_ref.as_deref(),
        // The server reads these only for a render-only push; with a binary the
        // plan's own embedded state is the authority, so nothing is claimed.
        prior_serial: args.prior_serial.map(|s| s as i64),
        prior_lineage: args.prior_lineage.as_deref(),
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
        Err(e) => return fail(cli, &explain(PlanOp::Push, &e, machine_identity(&ctx))),
    };

    // The stored binary must be the one that was sent, before anything is said
    // about a verdict on it.
    let sent_sha = tfplan_bytes.as_deref().map(sha256_hex);
    if let Err(x) = verify_stored_sha(sent_sha.as_deref(), result.tfplan_sha256.as_deref()) {
        return fail(cli, &x);
    }

    // The push response carries the verdict and the risk score, but not the
    // indicators behind them or how the plan was run, so the version's detail is
    // read back for those. Best effort: the verdict, the risk line and the exit
    // code never depend on it.
    let version_detail: Option<GetPlanVersionResponse> =
        result.version_id.as_deref().and_then(|id| {
            client
                .plan_version(&backend.org, &backend.workspace, id)
                .ok()
        });

    let gate = plan_view::push_gate(&result);
    // Asked to be the gating plan and stored under a ref instead: said on stderr
    // in every output mode, because a pipeline that expected to gate the
    // workspace and did not is about to find its apply has nothing to pull.
    if let Standing::Demoted(stored) = plan_view::push_standing(&result, args.plan_ref.as_deref()) {
        eprintln!(
            "warning: this plan does NOT gate workspace `{}`: the server stored it under {} because this pipeline's ref may not push state (see the verdict)",
            backend.workspace,
            match stored {
                Some(r) => format!("ref {r}"),
                None => "a ref".to_string(),
            }
        );
    }
    if json {
        let out = PushOutput {
            org: &backend.org,
            workspace: &backend.workspace,
            tfplan: tfplan_path.as_ref().map(|p| p.display().to_string()),
            gate,
            result: &result,
            risk: version_detail.as_ref().and_then(|d| d.risk.as_ref()),
            indicators: version_detail
                .as_ref()
                .and_then(|d| d.indicators.as_deref()),
        };
        output::print_structured(&out, cli.global.output.unwrap_or_default())?;
    } else {
        detail(&stored_line(&result));
        if let Some(sha) = sent_sha.as_deref() {
            detail(&format!("binary sha256 {sha} (stored as sent)"));
        }
        if result.stale == Some(true) {
            eprintln!(
                "{}",
                if tfplan_bytes.is_some() {
                    "warning: stored as stale: it is on record, but it cannot be applied, cannot lift a block, and is only pulled with --allow-stale"
                } else {
                    "warning: the server flagged this plan stale: the workspace is not at the state this plan says it was made from"
                }
            );
        }
        step("Verdict");
        let ctx_view = PushContext {
            workspace: &backend.workspace,
            requested_ref: args.plan_ref.as_deref(),
            has_binary: tfplan_bytes.is_some(),
        };
        for line in plan_view::push_verdict_lines(&result, version_detail.as_ref(), ctx_view, style)
        {
            detail(&line);
        }
        if let Some(h) = result.attestation_hash.as_deref() {
            detail(&format!("attestation {h}"));
        }
    }
    Ok(if gate == Gate::Blocked { EXIT_GATE } else { 0 })
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

/// Normalise an `--expect-sha` value: 64 hex characters, with an optional
/// `sha256:` prefix (the form `sha256sum` output is often wrapped in), lowercased.
pub fn parse_expect_sha(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let s = s
        .strip_prefix("sha256:")
        .or_else(|| s.strip_prefix("SHA256:"))
        .unwrap_or(s);
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(format!(
            "--expect-sha takes a sha-256 as 64 hex characters (as `sha256sum tfplan` prints it); got {} character(s)",
            s.chars().count()
        ))
    }
}

/// The pulled binary must be the one the caller expected. `expected` is already
/// normalised by [`parse_expect_sha`].
pub fn check_expected_sha(
    actual: &str,
    expected: Option<&str>,
    plan_id: Option<&str>,
) -> Result<(), Explained> {
    match expected {
        None => Ok(()),
        Some(e) if e.eq_ignore_ascii_case(actual) => Ok(()),
        Some(e) => Err(Explained::local(
            "plan_sha_mismatch",
            "the plan the server returned is not the plan that was expected",
            vec![
                format!("Expected sha256 {e} (--expect-sha)."),
                format!(
                    "Got      sha256 {actual} (plan {}).",
                    plan_id.unwrap_or("id not stated")
                ),
                "Another plan has been pushed here since the expected one - usually a newer pipeline run, or a re-plan. Applying it would apply something this run did not produce."
                    .to_string(),
                "Re-run the pipeline from its plan step, or pull the expected version by id with --id."
                    .to_string(),
            ],
            1,
        )),
    }
}

/// Make sure no plan is left at `path` after a pull whose bytes failed a hash
/// check. The pull would have overwritten whatever was there, and by now it is
/// established that the plan wanted at this path is not the one available - so an
/// older plan under the name the apply step reads is the one outcome that must
/// not survive. (A pull the server refused, or that never connected, touches
/// nothing: it has learned nothing about the file already there.) Returns the
/// sentence to add to the error.
fn clear_output(path: &Path) -> String {
    if !path.exists() {
        return "Nothing was written.".to_string();
    }
    match std::fs::remove_file(path) {
        Ok(()) => format!(
            "Nothing was written, and the existing {} was removed so that an older plan is not applied in its place.",
            path.display()
        ),
        Err(e) => format!(
            "Nothing was written, but the existing {} could not be removed ({e}): do NOT apply it.",
            path.display()
        ),
    }
}

fn pull(cli: &Cli, args: &PlanPullArgs) -> CmdResult {
    let json = is_json(cli);
    let tty = stderr_tty();
    let style = Style::detect_stderr(cli.global.no_color);
    let dir = working_dir(args.chdir.as_deref());

    let expected = match args.expect_sha.as_deref().map(parse_expect_sha) {
        Some(Ok(e)) => Some(e),
        Some(Err(msg)) => {
            eprintln!("error: {msg}");
            return Ok(EXIT_USAGE);
        }
        None => None,
    };

    let (backend, ctx) = match backend_for(cli, &dir, args.address.as_deref()) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let client = match auth::api_client(&ctx) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_AUTH);
        }
    };
    let selector = match args.id.as_deref() {
        Some(id) => PlanSelector::Id(id),
        None => PlanSelector::Latest {
            plan_ref: args.plan_ref.as_deref(),
        },
    };
    let out_path = {
        let p = PathBuf::from(args.out.as_deref().unwrap_or(DEFAULT_OUT));
        if p.is_absolute() {
            p
        } else {
            dir.join(p)
        }
    };
    if !json {
        step(&format!(
            "Pulling {} ({})",
            match selector {
                PlanSelector::Id(id) => format!("plan {id}"),
                PlanSelector::Latest { plan_ref: Some(r) } => format!("latest plan for ref {r}"),
                PlanSelector::Latest { plan_ref: None } =>
                    "the workspace's latest gating plan".into(),
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
        Err(e) => return fail(cli, &explain(PlanOp::Pull, &e, machine_identity(&ctx))),
    };

    // Two checks before a byte reaches the disk: the body is what the server
    // says it sent, and it is what the caller said to expect.
    let sha = match verify_sha(&pulled.bytes, pulled.tfplan_sha256.as_deref()) {
        Ok(s) => s,
        Err(msg) => {
            let x = Explained::local(
                "plan_sha_mismatch",
                "the download does not match the hash the server sent with it",
                vec![msg, clear_output(&out_path)],
                1,
            );
            return fail(cli, &x);
        }
    };
    if let Err(mut x) = check_expected_sha(&sha, expected.as_deref(), pulled.plan_id.as_deref()) {
        x.hint.push(clear_output(&out_path));
        return fail(cli, &x);
    }
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
            "{} {} · {} bytes · sha256 verified{}",
            Gate::Pass.mark(style),
            out_path.display(),
            pulled.bytes.len(),
            if expected.is_some() {
                " and equal to --expect-sha"
            } else {
                ""
            }
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
        Err(e) => return fail(cli, &explain(PlanOp::Read, &e, false)),
    };
    let rows = plan_rows(&versions);
    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&rows, format)? {
        return Ok(0);
    }
    print!("{}", render_ls(&rows));
    Ok(0)
}

/// The `-o json` shape of `plan show`: the version detail as the server sent it,
/// plus the two further reads the page is made of. Each is `null` if that read
/// failed, which is not the same as there being none.
#[derive(Debug, Serialize)]
struct ShowOutput<'a> {
    #[serde(flatten)]
    detail: &'a GetPlanVersionResponse,
    policy_results: Option<&'a [PlanPolicyResult]>,
    approvals: Option<&'a ListPlanVersionApprovalsResponse>,
}

/// A secondary read for `plan show`. A failure costs that section, not the page.
fn secondary<T>(what: &str, read: Result<T, ApiError>) -> Option<T> {
    match read {
        Ok(v) => Some(v),
        Err(e) => {
            let x = explain(PlanOp::Read, &e, false);
            eprintln!(
                "note: could not read the plan's {what}: {}{}",
                x.explanation,
                x.error.map(|c| format!(" ({c})")).unwrap_or_default()
            );
            None
        }
    }
}

fn show(cli: &Cli, workspace: &str, version_id: &str) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let detail_resp = match client.plan_version(&org, workspace, version_id) {
        Ok(v) => v,
        Err(e) => return fail(cli, &explain(PlanOp::Read, &e, false)),
    };
    let policy = secondary(
        "policy results",
        client.plan_policy_results(&org, workspace, version_id),
    );
    let approvals = secondary(
        "approvals",
        client.plan_approvals(&org, workspace, version_id),
    );

    let format = cli.global.output.unwrap_or_default();
    let structured = ShowOutput {
        detail: &detail_resp,
        policy_results: policy.as_deref(),
        approvals: approvals.as_ref(),
    };
    if output::print_structured(&structured, format)? {
        return Ok(0);
    }
    let style = Style::detect(cli.global.no_color);
    match plan_view::render_show(&detail_resp, policy.as_deref(), approvals.as_ref(), style) {
        Some(text) => {
            print!("{text}");
            Ok(0)
        }
        None => {
            eprintln!("error: the server returned no version summary for {version_id}");
            Ok(1)
        }
    }
}

// ── plan approve ────────────────────────────────────────────────────────────

/// Check an approval reason before sending it: it is required here (an approval
/// with no stated reason is no use to whoever reads the audit log), and the
/// server refuses one over 500 characters.
pub fn validate_reason(raw: &str) -> Result<&str, String> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(
            "--reason must say why this plan is approved; it is kept in the audit log".into(),
        );
    }
    let len = reason.chars().count();
    if len > MAX_REASON_CHARS {
        return Err(format!(
            "--reason is {len} characters; the limit is {MAX_REASON_CHARS}"
        ));
    }
    Ok(reason)
}

/// The refusal for a machine identity, made before any request: the server
/// would say the same, in fewer words, after a round trip.
fn machine_cannot_approve(mode: &str) -> Explained {
    Explained::local(
        "forbidden",
        format!("a plan is approved by a person, and this is a machine identity ({mode})"),
        vec![
            "A CI or bot token cannot approve: the pipeline that pushed a plan must not be able to wave it through."
                .to_string(),
            "Sign in as yourself (`terramantle auth login`, with no TERRAMANTLE_TOKEN or client credentials in the environment) and approve from there, or use the portal."
                .to_string(),
        ],
        EXIT_AUTH,
    )
}

fn approve(cli: &Cli, workspace: &str, version_id: &str, reason: &str) -> CmdResult {
    let reason = match validate_reason(reason) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(EXIT_USAGE);
        }
    };
    let ctx = match auth::auth_context(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_AUTH);
        }
    };
    if machine_identity(&ctx) {
        return fail(cli, &machine_cannot_approve(auth::mode_label(&ctx)));
    }
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let json = is_json(cli);
    if !json {
        step(&format!(
            "Approving plan {version_id} ({})",
            auth::mode_label(&ctx)
        ));
        detail(&format!("org {org} · workspace {workspace}"));
    }
    let resp = match client.plan_approve(&org, workspace, version_id, reason) {
        Ok(r) => r,
        // A machine identity never gets this far, so a `forbidden` here is about
        // the person's role.
        Err(e) => return fail(cli, &explain(PlanOp::Approve, &e, false)),
    };
    if json {
        output::print_structured(&resp, cli.global.output.unwrap_or_default())?;
    } else {
        let style = Style::detect_stderr(cli.global.no_color);
        for line in plan_view::approval_lines(&resp, style) {
            detail(&line);
        }
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

    // ── pre-flight, end to end over a real plan ─────────────────────────────

    const TOFU_FRESH: &[u8] = include_bytes!("../tests/fixtures/tfplan/tofu-fresh.tfplan");
    const TOFU_APPLIED: &[u8] = include_bytes!("../tests/fixtures/tfplan/tofu-applied.tfplan");

    fn pointer(lineage: &str, serial: i64) -> StatePointer {
        StatePointer {
            serial,
            lineage: Some(lineage.to_string()),
        }
    }

    /// A real plan against the state it was made from is current; move the
    /// serial and it is refused, with exit 7 and the server's own code, and the
    /// refusal says nothing was uploaded.
    #[test]
    fn a_real_plan_is_refused_once_the_workspace_moves_on() {
        let prior = tfplan_zip::read_prior_state(TOFU_APPLIED).unwrap();
        let lineage = prior.lineage.clone().unwrap();
        let serial = prior.serial.unwrap();

        let same = plan_preflight::check(&prior, Some(&pointer(&lineage, serial)));
        assert_eq!(
            preflight_action(&same, false, "prod", "tofu"),
            PreflightAction::Proceed
        );

        let moved = plan_preflight::check(&prior, Some(&pointer(&lineage, serial + 2)));
        let PreflightAction::Refuse(x) = preflight_action(&moved, false, "prod", "tofu") else {
            panic!("a stale plan must be refused");
        };
        assert_eq!(x.exit_code, 7);
        assert_eq!(x.error.as_deref(), Some("plan_stale"));
        assert_eq!(x.status, None, "the server was never asked");
        let text = x.lines().join("\n");
        assert!(text.starts_with("error: this plan is out of date"));
        assert!(text.contains(&format!("serial {serial}")));
        assert!(text.contains(&format!("serial {}", serial + 2)));
        assert!(text.contains("Nothing was uploaded"));
        assert!(text.contains("--allow-stale"));

        let foreign = plan_preflight::check(&prior, Some(&pointer("some-other-lineage", serial)));
        let PreflightAction::Refuse(x) = preflight_action(&foreign, false, "prod", "tofu") else {
            panic!("a foreign lineage must be refused");
        };
        assert_eq!(x.exit_code, 7);
        assert_eq!(x.error.as_deref(), Some("lineage_mismatch"));
        assert!(x.lines().join("\n").contains("two different states"));
    }

    /// A plan made from real state (it has a lineage) pushed to a workspace with
    /// NO state on the server: accepted, with no --allow-stale. That is the
    /// passive-drift shape - state that lives elsewhere - and the server never
    /// calls it stale, so neither may the pre-flight.
    #[test]
    fn a_real_plan_with_a_lineage_passes_when_the_workspace_has_no_state() {
        let prior = tfplan_zip::read_prior_state(TOFU_APPLIED).unwrap();
        assert!(prior.lineage.is_some(), "the fixture must carry a lineage");
        assert_eq!(
            preflight_action(&plan_preflight::check(&prior, None), false, "prod", "tofu"),
            PreflightAction::Proceed
        );
    }

    #[test]
    fn a_real_first_plan_is_current_only_while_the_workspace_has_no_state() {
        let prior = tfplan_zip::read_prior_state(TOFU_FRESH).unwrap();
        assert_eq!(
            preflight_action(&plan_preflight::check(&prior, None), false, "prod", "tofu"),
            PreflightAction::Proceed
        );
        // Even at the very serial the plan carries.
        let now = pointer("minted-since", prior.serial.unwrap());
        let PreflightAction::Refuse(x) = preflight_action(
            &plan_preflight::check(&prior, Some(&now)),
            false,
            "prod",
            "tofu",
        ) else {
            panic!("a plan from before the first state must be refused");
        };
        assert_eq!(x.error.as_deref(), Some("plan_stale"));
        assert!(x.explanation.contains("before the workspace had any state"));
    }

    /// `--allow-stale` turns the refusal into a warning and lets the push go on.
    #[test]
    fn allow_stale_pushes_for_the_record_with_a_warning() {
        let pre = Preflight::StaleSerial {
            plan: 4,
            current: 6,
        };
        let PreflightAction::ProceedForTheRecord(lines) =
            preflight_action(&pre, true, "prod", "terraform")
        else {
            panic!("--allow-stale must not refuse");
        };
        assert!(lines[0].starts_with("warning: this plan is out of date"));
        assert!(lines[0].ends_with("(plan_stale)"));
        assert!(lines.last().unwrap().contains("--allow-stale"));
        assert!(lines.last().unwrap().contains("cannot be applied"));
        // A current plan is unaffected by the flag.
        assert_eq!(
            preflight_action(&Preflight::Current, true, "prod", "terraform"),
            PreflightAction::Proceed
        );
    }

    // ── after the upload ────────────────────────────────────────────────────

    #[test]
    fn the_stored_binary_must_be_the_one_sent() {
        let sent = sha256_hex(b"plan bytes");
        assert!(verify_stored_sha(Some(&sent), Some(&sent)).is_ok());
        assert!(verify_stored_sha(Some(&sent), Some(&sent.to_uppercase())).is_ok());
        // Render-only: nothing was sent, so nothing to compare.
        assert!(verify_stored_sha(None, None).is_ok());

        let other = sha256_hex(b"different");
        let x = verify_stored_sha(Some(&sent), Some(&other)).unwrap_err();
        assert_eq!(x.exit_code, 1, "a hard error, not a gate verdict");
        assert_eq!(x.error.as_deref(), Some("tfplan_sha_mismatch"));
        let text = x.lines().join("\n");
        assert!(text.contains(&sent) && text.contains(&other));
        assert!(text.contains("do not apply or approve it"));

        // A binary was sent and the server says it has none.
        let x = verify_stored_sha(Some(&sent), None).unwrap_err();
        assert!(x.lines().join("\n").contains("recorded no binary"));
    }

    #[test]
    fn a_render_only_push_is_warned_about_what_it_cannot_do() {
        let bare = render_only_warning(false).join("\n");
        assert!(bare.starts_with("warning:"));
        assert!(bare.contains("cannot verify what state this plan was made from"));
        assert!(bare.contains("can never lift one or satisfy \"require a passing plan\""));

        let claimed = render_only_warning(true).join("\n");
        assert!(claimed.starts_with("note:"));
        assert!(claimed.contains("claim"));
        assert!(claimed.contains("can never lift one"));
    }

    /// `deduped` is said only when the server says it, and a re-push that was
    /// judged afresh reads like any other push.
    #[test]
    fn the_stored_line_only_claims_the_same_version_when_deduped() {
        let result = |deduped: bool| -> PlanPushResult {
            serde_json::from_value(serde_json::json!({
                "ok": true, "version_id": "v-1", "attestation_hash": "ab", "tfplan_sha256": "cd",
                "prior_serial": 4, "prior_lineage": "l", "lineage_source": "tfplan",
                "stale": false, "deduped": deduped,
                "verdict": { "blocked": false, "reasons": [], "sources": [], "warnings": [] },
                "scan_status": "pending"
            }))
            .unwrap()
        };
        assert_eq!(
            stored_line(&result(false)),
            "version v-1 · prior serial 4 (from the plan)"
        );
        assert!(stored_line(&result(true))
            .ends_with("the same version as before (this exact plan was already pushed)"));
    }

    // ── pull ────────────────────────────────────────────────────────────────

    #[test]
    fn expect_sha_is_validated_before_anything_is_fetched() {
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(parse_expect_sha(good).unwrap(), good);
        assert_eq!(parse_expect_sha(&good.to_uppercase()).unwrap(), good);
        assert_eq!(parse_expect_sha(&format!("sha256:{good}\n")).unwrap(), good);
        assert!(parse_expect_sha("deadbeef").is_err());
        assert!(parse_expect_sha("").is_err());
        assert!(parse_expect_sha(&"g".repeat(64)).is_err());
        // The common slip: passing `sha256sum`'s whole line, filename and all.
        assert!(parse_expect_sha(&format!("{good}  tfplan")).is_err());
    }

    #[test]
    fn a_pull_that_is_not_the_expected_plan_fails_clearly() {
        let actual = sha256_hex(b"what the server sent");
        let expected = sha256_hex(b"what the plan job made");
        assert!(check_expected_sha(&actual, None, Some("p1")).is_ok());
        assert!(check_expected_sha(&actual, Some(&actual), Some("p1")).is_ok());

        let x = check_expected_sha(&actual, Some(&expected), Some("p1")).unwrap_err();
        assert_ne!(x.exit_code, 0);
        assert_eq!(x.error.as_deref(), Some("plan_sha_mismatch"));
        let text = x.lines().join("\n");
        assert!(text.contains(&expected) && text.contains(&actual));
        assert!(text.contains("plan p1"));
        assert!(text.contains("not the plan that was expected"));
    }

    /// The file a failed pull leaves behind is none: not a new one, and not the
    /// one a previous pull put there.
    #[test]
    fn a_failed_pull_leaves_no_plan_at_the_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("tfplan");

        assert_eq!(clear_output(&out), "Nothing was written.");
        assert!(!out.exists());

        std::fs::write(&out, b"an older plan").unwrap();
        let said = clear_output(&out);
        assert!(!out.exists(), "the older plan must not survive");
        assert!(said.contains("was removed"));
    }

    // ── approve ─────────────────────────────────────────────────────────────

    #[test]
    fn an_approval_needs_a_reason_the_server_will_take() {
        assert_eq!(
            validate_reason("  reviewed, expected  ").unwrap(),
            "reviewed, expected"
        );
        assert!(validate_reason("").is_err());
        assert!(validate_reason("   ").is_err());
        assert!(validate_reason(&"x".repeat(500)).is_ok());
        let long = validate_reason(&"x".repeat(501)).unwrap_err();
        assert!(long.contains("501") && long.contains("500"));
        // Characters, not bytes: 500 two-byte characters are still within the limit.
        assert!(validate_reason(&"é".repeat(500)).is_ok());
    }

    #[test]
    fn a_machine_identity_is_told_why_it_cannot_approve() {
        let x = machine_cannot_approve("github oidc");
        assert_eq!(x.exit_code, 5);
        assert_eq!(x.error.as_deref(), Some("forbidden"));
        let text = x.lines().join("\n");
        assert!(text.contains("machine identity (github oidc)"));
        assert!(text.contains("auth login"));
    }

    #[test]
    fn machine_identity_follows_the_auth_mode() {
        let ctx = |mode| AuthContext {
            api_url: "https://x".into(),
            issuer_override: None,
            audience_override: None,
            mode,
        };
        assert!(machine_identity(&ctx(AuthMode::GitHub)));
        assert!(machine_identity(&ctx(AuthMode::GitLab)));
        assert!(machine_identity(&ctx(AuthMode::ClientCredentials)));
        assert!(!machine_identity(&ctx(AuthMode::Device)));
    }
}
