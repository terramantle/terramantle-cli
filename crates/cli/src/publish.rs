//! `modules changed` / `modules publish` / `state publish` — the repo-aware
//! publish surface (SCAFFOLD-PUBLISH-AUTH.md §5–§7).
//!
//! This is the thin IO shell over [`tm_release`]: change detection, versioning,
//! and deterministic packaging live in that crate (unit-tested, network-free);
//! here we only read the manifest, drive external tools (terraform-docs, git),
//! call the registry, and narrate.
//!
//! The load-bearing *decisions* — which artefacts to target and which version to
//! stamp — are kept in pure functions ([`select_targets`], [`plan_version`]) so
//! every branch is unit-tested with no git/network/filesystem.
//!
//! Human narration → stderr; machine output (`-o json`) → stdout stays clean.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use tm_release::{
    apply_bump, package_dir, plan_all, tag_name, ArtefactPlan, BumpLevel, GitRepo, PackageOptions,
    Version,
};
use tm_scaffold::{Artefact, Manifest};

use crate::auth;
use crate::cli::{BumpArg, Cli, DocsMode, ModulePublishArgs};
use crate::commands::CmdResult;
use crate::lock::{self, PollOutcome, PostureRow};
use crate::output::{self, TableView};

/// Exit 2: a usage / config error (bad manifest, missing `--provider`, non-semver
/// `--version`, unknown module) — SPEC §9.
const EXIT_USAGE: i32 = 2;
/// Exit 3: posture gate tripped (`--fail-on-atrisk`) — SPEC §9.
const EXIT_POSTURE_GATE: i32 = 3;
/// Exit 6: a targeted workspace has no `.terraform.lock.hcl` — SPEC §9.
const EXIT_NOT_FOUND: i32 = 6;

/// The canonical Terraform dependency-lock filename (mirrors `lock::push`).
const LOCK_FILE_NAME: &str = ".terraform.lock.hcl";

// ── pure decisions (unit-tested, no IO) ─────────────────────────────────────────

/// Select the artefact plans to act on (§6 step 2):
///   * explicit `--module <name>…` → exactly those, in order (unknown name errors),
///   * else `--all` → every artefact,
///   * else the changed set (never-tagged or with files changed since the last tag).
///
/// Returns `Err(name)` for the first `--module` name that doesn't resolve, which
/// the caller maps to exit 2.
pub fn select_targets<'a>(
    plans: &'a [ArtefactPlan],
    modules: &[String],
    all: bool,
) -> Result<Vec<&'a ArtefactPlan>, String> {
    if !modules.is_empty() {
        let mut out = Vec::with_capacity(modules.len());
        for name in modules {
            match plans.iter().find(|p| &p.name == name) {
                Some(p) => out.push(p),
                None => return Err(name.clone()),
            }
        }
        Ok(out)
    } else if all {
        Ok(plans.iter().collect())
    } else {
        Ok(plans.iter().filter(|p| p.changed).collect())
    }
}

/// Resolve the version to stamp for one artefact (§5 override + validation):
///   * `--version` wins verbatim (already strict-semver parsed by the caller),
///   * else `--bump` applies to the last tag (or `0.0.0` when never tagged),
///   * else the conventional-commit computation — but a `None` bump on an
///     already-released artefact means *nothing to release* → `None` (skip).
pub fn plan_version(
    plan: &ArtefactPlan,
    version_override: Option<&Version>,
    bump_override: Option<BumpLevel>,
) -> Option<Version> {
    if let Some(v) = version_override {
        return Some(v.clone());
    }
    if let Some(b) = bump_override {
        let base = plan
            .current_version
            .clone()
            .unwrap_or_else(|| Version::new(0, 0, 0));
        return Some(apply_bump(&base, b));
    }
    // Conventional: an unreleased artefact always ships (0.1.0 or its computed
    // next); a released one with no bump-worthy commits is skipped.
    if plan.bump == BumpLevel::None && plan.current_version.is_some() {
        return None;
    }
    Some(plan.next_version.clone())
}

/// Map the clap `--bump` value to the release-engine [`BumpLevel`].
fn bump_from_arg(arg: BumpArg) -> BumpLevel {
    match arg {
        BumpArg::Major => BumpLevel::Major,
        BumpArg::Minor => BumpLevel::Minor,
        BumpArg::Patch => BumpLevel::Patch,
    }
}

/// The lowercase label for a [`BumpLevel`] (table rendering; JSON uses serde).
fn bump_label(b: BumpLevel) -> &'static str {
    match b {
        BumpLevel::None => "none",
        BumpLevel::Patch => "patch",
        BumpLevel::Minor => "minor",
        BumpLevel::Major => "major",
    }
}

/// The `git -- <path>` argument for an artefact (`.` for a poly root).
fn git_path(rel_dir: &str) -> &str {
    if rel_dir.is_empty() || rel_dir == "." {
        "."
    } else {
        rel_dir
    }
}

// ── manifest loading ────────────────────────────────────────────────────────────

/// Read + parse `terramantle.hcl` at the repo root, mapping a missing file (the
/// command is repo-aware, like `upgrade`) or a parse error to exit 2.
fn load_manifest(root: &Path) -> Result<Manifest, i32> {
    let path = root.join(Manifest::FILENAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "error: no {} here — run `terramantle init` first",
                Manifest::FILENAME
            );
            return Err(EXIT_USAGE);
        }
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return Err(EXIT_USAGE);
        }
    };
    Manifest::parse(&text).map_err(|e| {
        eprintln!("error: {e}");
        EXIT_USAGE
    })
}

// ── modules changed (§5) ────────────────────────────────────────────────────────

/// The `-o json` row for `modules changed` (§5 CI contract — **stable**):
/// `{ name, path, last_version, bump, next_version, changed_files }`.
#[derive(Debug, Clone, Serialize)]
struct ChangedRow {
    name: String,
    path: String,
    last_version: Option<String>,
    bump: BumpLevel,
    next_version: Option<String>,
    changed_files: Vec<String>,
}

impl ChangedRow {
    /// Build a row from a plan, resolving the change files from git and the
    /// releasable next version (null when a released artefact has no bump).
    fn from_plan(root: &Path, plan: &ArtefactPlan) -> Self {
        let git = GitRepo::new(root);
        let changed_files = git
            .changed_files(plan.last_tag.as_deref(), git_path(&plan.rel_dir))
            .unwrap_or_default();
        let releasable = plan.bump != BumpLevel::None || plan.current_version.is_none();
        ChangedRow {
            name: plan.name.clone(),
            path: plan.rel_dir.clone(),
            last_version: plan.current_version.as_ref().map(Version::to_string),
            bump: plan.bump,
            next_version: releasable.then(|| plan.next_version.to_string()),
            changed_files,
        }
    }
}

/// `terramantle modules changed [--all]`.
pub fn changed(cli: &Cli, all: bool) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest = match load_manifest(&root) {
        Ok(m) => m,
        Err(code) => return Ok(code),
    };
    if manifest.artefact != Artefact::Modules {
        eprintln!("error: `modules changed` requires an artefact=modules repo");
        return Ok(EXIT_USAGE);
    }

    let plans = match plan_all(&root, &manifest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(1);
        }
    };

    let rows: Vec<ChangedRow> = plans
        .iter()
        .filter(|p| all || p.changed)
        .map(|p| ChangedRow::from_plan(&root, p))
        .collect();

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&rows, format)? {
        return Ok(0);
    }

    let mut view = TableView::new(["name", "last", "bump", "next", "changed"]);
    for r in &rows {
        view.row([
            r.name.clone(),
            r.last_version.clone().unwrap_or_else(|| "—".to_string()),
            bump_label(r.bump).to_string(),
            r.next_version.clone().unwrap_or_else(|| "—".to_string()),
            r.changed_files.len().to_string(),
        ]);
    }
    println!("{}", view.render());
    Ok(0)
}

// ── modules publish (§6) ────────────────────────────────────────────────────────

/// One module's publish outcome (`-o json`).
#[derive(Debug, Clone, Serialize)]
struct PublishRow {
    name: String,
    path: String,
    /// `published` | `would-publish` (dry-run) | `skipped`.
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// The `-o json` summary for `modules publish`.
#[derive(Debug, Clone, Serialize)]
struct PublishSummary {
    dry_run: bool,
    provider: String,
    modules: Vec<PublishRow>,
}

/// `terramantle modules publish …`.
pub fn publish(cli: &Cli, args: &ModulePublishArgs) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest = match load_manifest(&root) {
        Ok(m) => m,
        Err(code) => return Ok(code),
    };
    if manifest.artefact != Artefact::Modules {
        eprintln!("error: `modules publish` requires an artefact=modules repo");
        return Ok(EXIT_USAGE);
    }

    // provider is not carried by the manifest (name = dir basename), so it must be
    // supplied explicitly (§6 step 4).
    let Some(provider) = args.provider.as_deref() else {
        eprintln!("error: --provider is required (module name = directory basename)");
        return Ok(EXIT_USAGE);
    };

    // --version is validated as strict semver2 up front; a bad value is exit 2.
    let version_override = match &args.version {
        Some(v) => match tm_release::parse_strict(v) {
            Ok(parsed) => Some(parsed),
            Err(e) => {
                eprintln!("error: {e}");
                return Ok(EXIT_USAGE);
            }
        },
        None => None,
    };
    let bump_override = args.bump.map(bump_from_arg);

    let plans = match plan_all(&root, &manifest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(1);
        }
    };
    let targets = match select_targets(&plans, &args.modules, args.all) {
        Ok(t) => t,
        Err(name) => {
            eprintln!("error: no module '{name}' in this repo");
            return Ok(EXIT_USAGE);
        }
    };
    if targets.is_empty() {
        eprintln!("nothing to publish (no changed modules; pass --all or --module)");
        return finish_publish(cli, args.dry_run, provider, Vec::new());
    }

    let run_docs = !matches!(args.docs, Some(DocsMode::Skip)) && manifest.ci.terraform_docs;

    // The client + org are only needed for a real upload; a dry-run stays offline.
    let client_org = if args.dry_run {
        None
    } else {
        match crate::discovery::client_and_org(cli) {
            Ok(v) => Some(v),
            Err(code) => return Ok(code),
        }
    };

    let mut rows: Vec<PublishRow> = Vec::new();
    let mut exit = 0;

    for plan in targets {
        let Some(version) = plan_version(plan, version_override.as_ref(), bump_override) else {
            eprintln!("==> {} · skipped (no release-worthy commits)", plan.name);
            rows.push(PublishRow {
                name: plan.name.clone(),
                path: plan.rel_dir.clone(),
                action: "skipped",
                version: None,
                tag: None,
                sha256: None,
                reason: Some("no bump".to_string()),
            });
            continue;
        };

        let dir = root.join(&plan.rel_dir);
        // The tag this version maps to — created by `terramantle repo tag`, not
        // here: publish uploads the artefact, `repo tag` stamps the git tags.
        let tag = tag_name(manifest.structure, &plan.name, &version);
        eprintln!("==> {} · {version} ({tag})", plan.name);

        if run_docs {
            regen_docs(&root, &dir);
        }

        // Deterministic package → stable sha256.
        let (tarball, sha) = match package_dir(&dir, &PackageOptions::default()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: packaging {} failed: {e}", plan.name);
                return Ok(1);
            }
        };
        let tarball_name = format!("{}-{version}.tar.gz", plan.name);
        eprintln!("    packaged {tarball_name} · sha256 {sha}");

        // Dry-run stays offline — report intent and stop.
        if args.dry_run {
            eprintln!("    dry-run · would upload");
            rows.push(PublishRow {
                name: plan.name.clone(),
                path: plan.rel_dir.clone(),
                action: "would-publish",
                version: Some(version.to_string()),
                tag: Some(tag),
                sha256: Some(sha),
                reason: None,
            });
            continue;
        }

        let (client, org) = client_org
            .as_ref()
            .expect("client resolved for non-dry-run");
        let ack = client.module_publish(
            org,
            &plan.name,
            provider,
            &version.to_string(),
            &tarball,
            args.description.as_deref(),
        );
        match ack {
            Ok(resp) => {
                eprintln!(
                    "    uploaded · status {} (tag {tag} on `terramantle repo tag`)",
                    resp.status.as_deref().unwrap_or("ok")
                );
                rows.push(PublishRow {
                    name: plan.name.clone(),
                    path: plan.rel_dir.clone(),
                    action: "published",
                    version: Some(version.to_string()),
                    tag: Some(tag),
                    sha256: Some(sha),
                    reason: None,
                });
            }
            Err(e) => {
                eprintln!("error: publishing {} failed: {e}", plan.name);
                exit = e.exit_code();
            }
        }
    }

    finish_publish(cli, args.dry_run, provider, rows)
        .map(|code| if exit != 0 { exit } else { code })
}

/// Emit the `-o json` publish summary (if requested) and return exit 0.
fn finish_publish(cli: &Cli, dry_run: bool, provider: &str, rows: Vec<PublishRow>) -> CmdResult {
    let format = cli.global.output.unwrap_or_default();
    let summary = PublishSummary {
        dry_run,
        provider: provider.to_string(),
        modules: rows,
    };
    output::print_structured(&summary, format)?;
    Ok(0)
}

/// Regenerate the module README between the terraform-docs markers, best-effort:
/// probe `terraform-docs` on PATH and run it, else print an actionable hint.
fn regen_docs(root: &Path, dir: &Path) {
    if !on_path("terraform-docs") {
        eprintln!(
            "    terraform-docs not on PATH — skipping README regen (install: https://terraform-docs.io, or pass --docs skip)"
        );
        return;
    }
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    let status = Command::new("terraform-docs")
        .current_dir(root)
        .args([
            "markdown",
            "table",
            "--output-file",
            "README.md",
            "--output-mode",
            "inject",
        ])
        .arg(rel)
        .status();
    match status {
        Ok(s) if s.success() => eprintln!("    regenerated README.md"),
        _ => eprintln!("    warning: terraform-docs failed; leaving README.md as-is"),
    }
}

/// Create + push the release tag, best-effort (the upload already succeeded, so a
/// tag/push failure warns rather than aborting the whole run).
fn create_and_push_tag(root: &Path, tag: &str) {
    let created = run_git(root, &["tag", tag]);
    if let Err(e) = created {
        eprintln!("    warning: could not create tag {tag}: {e}");
        return;
    }
    match run_git(root, &["push", "origin", tag]) {
        Ok(()) => eprintln!("    tagged + pushed {tag}"),
        Err(e) => eprintln!("    warning: could not push tag {tag}: {e}"),
    }
}

/// Run `git -C <root> <args>`, returning the stderr on a non-zero exit.
fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Whether `bin` is an executable file on `PATH` (probe, no side effects).
fn on_path(bin: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

/// Whether an exact git tag already exists locally.
fn tag_exists(root: &Path, tag: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["tag", "--list", tag])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

// ── repo tag (mono release tagging) ───────────────────────────────────────────────

/// One module's `repo tag` outcome (`-o json`).
#[derive(Debug, Clone, Serialize)]
struct RepoTagRow {
    name: String,
    /// `tagged` | `would-tag` (dry-run) | `exists` (idempotent skip) | `skipped`.
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

/// The `-o json` summary for `repo tag`.
#[derive(Debug, Clone, Serialize)]
struct RepoTagSummary {
    dry_run: bool,
    tags: Vec<RepoTagRow>,
}

/// `terramantle repo tag [--dry-run]` (SCAFFOLD-PUBLISH-AUTH.md §5).
///
/// The mono-repo release tagger, meant to run on the default branch after a
/// successful publish: for each module it works out the next version from the
/// `<module>@<semver>` tags (first release → `<module>@1.0.0`, thereafter driven
/// by Conventional Commits), packages the module deterministically, then creates
/// and pushes the `<module>@<version>` git tag. Already-existing tags and modules
/// with no release-worthy change are skipped, so it is safe to re-run.
pub fn repo_tag(cli: &Cli, dry_run: bool) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest = match load_manifest(&root) {
        Ok(m) => m,
        Err(code) => return Ok(code),
    };
    if manifest.artefact != Artefact::Modules {
        eprintln!("error: `repo tag` versions modules; this repo is artefact=workspaces");
        return Ok(EXIT_USAGE);
    }

    let plans = match plan_all(&root, &manifest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(1);
        }
    };

    let mut rows: Vec<RepoTagRow> = Vec::new();
    for plan in &plans {
        // Same version computation as publish: first release is 1.0.0, an already
        // released module with no bump-worthy commits is skipped.
        let Some(version) = plan_version(plan, None, None) else {
            eprintln!("==> {} · skipped (no release-worthy commits)", plan.name);
            rows.push(RepoTagRow {
                name: plan.name.clone(),
                action: "skipped",
                tag: None,
                sha256: None,
            });
            continue;
        };
        let tag = tag_name(manifest.structure, &plan.name, &version);

        // Idempotent: never re-tag an existing version (safe re-runs on main).
        if tag_exists(&root, &tag) {
            eprintln!("==> {} · {tag} already exists — skipping", plan.name);
            rows.push(RepoTagRow {
                name: plan.name.clone(),
                action: "exists",
                tag: Some(tag),
                sha256: None,
            });
            continue;
        }

        // Package the module (the version drives the artefact) before tagging, so a
        // tag only ever stamps a module that packages cleanly + reproducibly.
        let dir = root.join(&plan.rel_dir);
        let (_tarball, sha) = match package_dir(&dir, &PackageOptions::default()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: packaging {} failed: {e}", plan.name);
                return Ok(1);
            }
        };
        eprintln!("==> {} · {version} ({tag}) · sha256 {sha}", plan.name);

        if dry_run {
            eprintln!("    dry-run · would tag + push {tag}");
            rows.push(RepoTagRow {
                name: plan.name.clone(),
                action: "would-tag",
                tag: Some(tag),
                sha256: Some(sha),
            });
            continue;
        }

        create_and_push_tag(&root, &tag);
        rows.push(RepoTagRow {
            name: plan.name.clone(),
            action: "tagged",
            tag: Some(tag),
            sha256: Some(sha),
        });
    }

    // For json/yaml this prints the machine summary; in table mode it returns
    // false and the per-module lines narrated above are the output.
    let format = cli.global.output.unwrap_or_default();
    output::print_structured(
        &RepoTagSummary {
            dry_run,
            tags: rows,
        },
        format,
    )?;
    Ok(0)
}

// ── state publish (§7) ──────────────────────────────────────────────────────────

/// Arguments for `state publish`, unpacked from the clap subcommand.
pub struct StatePublishArgs<'a> {
    pub workspaces: &'a [String],
    pub all: bool,
    pub fail_on_atrisk: bool,
    pub posture_timeout: u64,
    pub require_posture: bool,
    pub repo_url: Option<&'a str>,
}

/// One workspace's publish outcome (`-o json`).
#[derive(Debug, Clone, Serialize)]
struct StatePublishRow {
    workspace: String,
    path: String,
    ok: bool,
    providers_count: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    posture: Option<Vec<PostureRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    posture_status: Option<&'static str>,
}

/// The `-o json` summary for `state publish`.
#[derive(Debug, Clone, Serialize)]
struct StatePublishSummary {
    org: String,
    published: usize,
    workspaces: Vec<StatePublishRow>,
}

/// Resolve the (workspace-name, dir) units to publish. mono → discovery globs;
/// poly → the repo root, named from the resolved `--workspace`/config or the repo
/// basename. Only directories carrying a `.terraform.lock.hcl` qualify.
fn workspace_units(
    root: &Path,
    manifest: &Manifest,
    poly_name: Option<&str>,
) -> Result<Vec<(String, PathBuf)>, i32> {
    let arts = tm_release::artefacts(root, manifest).map_err(|e| {
        eprintln!("error: {e}");
        1
    })?;
    let mut out = Vec::new();
    for a in arts {
        let dir = root.join(&a.rel_dir);
        if !dir.join(LOCK_FILE_NAME).is_file() {
            continue;
        }
        let name = if a.rel_dir == "." {
            poly_name.map(str::to_string).unwrap_or(a.name)
        } else {
            a.name
        };
        out.push((name, dir));
    }
    Ok(out)
}

/// `terramantle state publish …`.
pub fn state_publish(cli: &Cli, args: &StatePublishArgs) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest = match load_manifest(&root) {
        Ok(m) => m,
        Err(code) => return Ok(code),
    };
    if manifest.artefact != Artefact::Workspaces {
        eprintln!("error: `state publish` requires an artefact=workspaces repo");
        return Ok(EXIT_USAGE);
    }

    let (client, org) = match crate::discovery::client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };

    let poly_name = auth::config_workspace(cli)?;
    let discovered = match workspace_units(&root, &manifest, poly_name.as_deref()) {
        Ok(u) => u,
        Err(code) => return Ok(code),
    };

    // Select: explicit names (each must resolve to a discovered lock), else all.
    let selected: Vec<(String, PathBuf)> = if args.workspaces.is_empty() {
        if discovered.is_empty() {
            eprintln!("no workspaces with a {LOCK_FILE_NAME} found");
        }
        discovered
    } else {
        let mut out = Vec::new();
        for name in args.workspaces {
            match discovered.iter().find(|(n, _)| n == name) {
                Some(unit) => out.push(unit.clone()),
                None => {
                    eprintln!("error: workspace '{name}' has no {LOCK_FILE_NAME}");
                    return Ok(EXIT_NOT_FOUND);
                }
            }
        }
        out
    };
    let _ = args.all; // selection is name-list-or-all; `--all` documents intent.

    let repo_url = args
        .repo_url
        .map(str::to_string)
        .or_else(|| tm_scaffold::vcs::origin_remote(&root));

    let mut rows: Vec<StatePublishRow> = Vec::new();
    let mut published = 0usize;
    let mut exit = 0;

    for (name, dir) in &selected {
        let lock_path = dir.join(LOCK_FILE_NAME);
        let bytes = match std::fs::read(&lock_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", lock_path.display());
                return Ok(EXIT_NOT_FOUND);
            }
        };
        let providers = lock::parse_provider_addresses(&String::from_utf8_lossy(&bytes));
        eprintln!("==> {name} · {} providers", providers.len());

        let resp = match client.lock_push(&org, name, &bytes, repo_url.as_deref()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {name}: {e}");
                exit = e.exit_code();
                continue;
            }
        };
        published += 1;
        for w in &resp.warnings {
            eprintln!("    warning: {w}");
        }

        let (posture, posture_status) = if args.fail_on_atrisk {
            match aggregate_posture(&client, &org, name, &providers, args.posture_timeout) {
                PostureOutcome::Ready(rows) => {
                    if rows.iter().any(PostureRow::is_at_risk) {
                        exit = EXIT_POSTURE_GATE;
                    }
                    (Some(rows), None)
                }
                PostureOutcome::Unknown(status) => {
                    if args.require_posture {
                        exit = EXIT_POSTURE_GATE;
                    }
                    (None, Some(status))
                }
            }
        } else {
            (None, None)
        };

        eprintln!("    pushed · {} providers", resp.providers_count);
        rows.push(StatePublishRow {
            workspace: name.clone(),
            path: dir
                .strip_prefix(&root)
                .unwrap_or(dir)
                .to_string_lossy()
                .into_owned(),
            ok: resp.ok,
            providers_count: resp.providers_count,
            warnings: resp.warnings,
            posture,
            posture_status,
        });
    }

    eprintln!("published {published}/{} workspace(s)", selected.len());

    let format = cli.global.output.unwrap_or_default();
    let summary = StatePublishSummary {
        org,
        published,
        workspaces: rows,
    };
    if !output::print_structured(&summary, format)? {
        // Table already narrated to stderr; nothing extra on stdout.
    }
    Ok(exit)
}

/// The result of aggregating one workspace's posture.
enum PostureOutcome {
    Ready(Vec<PostureRow>),
    Unknown(&'static str),
}

/// Poll workspace-providers for the pushed set (reusing `lock::poll_posture`) and
/// derive posture rows, or report why posture is unknown (timeout/error).
fn aggregate_posture(
    client: &tm_api::Client,
    org: &str,
    workspace: &str,
    providers: &[String],
    timeout_secs: u64,
) -> PostureOutcome {
    let start = std::time::Instant::now();
    let outcome = lock::poll_posture(
        providers,
        Duration::from_secs(timeout_secs),
        Duration::from_secs(1),
        || client.workspace_providers(org, workspace),
        std::thread::sleep,
        || start.elapsed(),
    );
    match outcome {
        Ok(PollOutcome::Ready(resp)) => {
            let overview = client.providers_overview(org).unwrap_or_default();
            PostureOutcome::Ready(lock::evaluate_posture(providers, &resp, &overview))
        }
        Ok(PollOutcome::TimedOut(_)) => PostureOutcome::Unknown("timed_out"),
        Err(e) => {
            eprintln!("    warning: could not read posture: {e}");
            PostureOutcome::Unknown("error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(
        name: &str,
        current: Option<(u64, u64, u64)>,
        bump: BumpLevel,
        changed: bool,
    ) -> ArtefactPlan {
        let current_version = current.map(|(a, b, c)| Version::new(a, b, c));
        let next_version = match &current_version {
            Some(v) => apply_bump(v, bump),
            None => Version::new(0, 1, 0),
        };
        ArtefactPlan {
            name: name.to_string(),
            rel_dir: format!("modules/{name}"),
            current_version,
            last_tag: None,
            changed,
            changed_file_count: usize::from(changed),
            bump,
            next_version,
        }
    }

    #[test]
    fn select_defaults_to_changed_set() {
        let plans = vec![
            plan("a", Some((1, 0, 0)), BumpLevel::Minor, true),
            plan("b", Some((2, 0, 0)), BumpLevel::None, false),
        ];
        let got = select_targets(&plans, &[], false).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "a");
    }

    #[test]
    fn select_all_includes_unchanged() {
        let plans = vec![
            plan("a", Some((1, 0, 0)), BumpLevel::Minor, true),
            plan("b", Some((2, 0, 0)), BumpLevel::None, false),
        ];
        assert_eq!(select_targets(&plans, &[], true).unwrap().len(), 2);
    }

    #[test]
    fn select_named_errors_on_unknown() {
        let plans = vec![plan("a", None, BumpLevel::None, true)];
        assert_eq!(
            select_targets(&plans, &["ghost".to_string()], false).unwrap_err(),
            "ghost"
        );
        let ok = select_targets(&plans, &["a".to_string()], false).unwrap();
        assert_eq!(ok.len(), 1);
    }

    #[test]
    fn version_override_wins() {
        let p = plan("a", Some((1, 2, 3)), BumpLevel::Minor, true);
        let v = Version::new(9, 9, 9);
        assert_eq!(plan_version(&p, Some(&v), Some(BumpLevel::Major)), Some(v));
    }

    #[test]
    fn bump_override_applies_to_last_tag() {
        let p = plan("a", Some((1, 2, 3)), BumpLevel::None, true);
        assert_eq!(
            plan_version(&p, None, Some(BumpLevel::Minor)),
            Some(Version::new(1, 3, 0))
        );
        // Never-tagged + explicit patch → 0.0.1 from the 0.0.0 base.
        let fresh = plan("b", None, BumpLevel::None, true);
        assert_eq!(
            plan_version(&fresh, None, Some(BumpLevel::Patch)),
            Some(Version::new(0, 0, 1))
        );
    }

    #[test]
    fn conventional_skips_released_with_no_bump() {
        let released = plan("a", Some((1, 0, 0)), BumpLevel::None, true);
        assert_eq!(plan_version(&released, None, None), None);

        let bumped = plan("b", Some((1, 0, 0)), BumpLevel::Minor, true);
        assert_eq!(
            plan_version(&bumped, None, None),
            Some(Version::new(1, 1, 0))
        );

        // Never-tagged always ships its initial version.
        let fresh = plan("c", None, BumpLevel::None, true);
        assert_eq!(
            plan_version(&fresh, None, None),
            Some(Version::new(0, 1, 0))
        );
    }

    #[test]
    fn bump_arg_maps_to_level() {
        assert_eq!(bump_from_arg(BumpArg::Major), BumpLevel::Major);
        assert_eq!(bump_from_arg(BumpArg::Minor), BumpLevel::Minor);
        assert_eq!(bump_from_arg(BumpArg::Patch), BumpLevel::Patch);
    }
}
