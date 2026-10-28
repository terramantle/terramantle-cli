//! `terramantle doctor` — read-only checks on a scaffolded repo.
//!
//! Every check here exists because its failure mode is *silent or misleading*
//! somewhere else:
//!
//! * No VCS trust entry → a correct pipeline 401s on its first push, and
//!   nothing in the repo explains why, because the missing state is on the
//!   server.
//! * A stale scaffold → CI runs an old pipeline that may not even reference the
//!   manifest's current settings.
//! * Un-generated Terramate code → the drift gate fails in CI rather than here.
//! * A missing `.terraform.lock.hcl` → `state publish` skips that workspace;
//!   before this release it then exited 0, so CI went green having published
//!   nothing.
//! * A backend address that disagrees with the directory basename → the lock
//!   file is uploaded to one workspace while the state lives in another.
//!
//! Nothing is written and nothing is fixed: the point is to name the problem
//! precisely enough that fixing it is obvious.

use std::path::Path;

use tm_scaffold::render::desired_files_with;
use tm_scaffold::{compute, Manifest, VcsProvider};

use crate::cli::{Cli, DoctorArgs};
use crate::commands::CmdResult;

/// Exit 3 when any check fails — a gate result, matching `upgrade --check`.
const EXIT_FINDINGS: i32 = 3;

#[derive(Debug, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
    /// Could not be determined (offline, unauthenticated, not applicable).
    Skip,
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Ok,
            detail: detail.into(),
        }
    }
    fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Warn,
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Fail,
            detail: detail.into(),
        }
    }
    fn skip(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Skip,
            detail: detail.into(),
        }
    }
    fn glyph(&self) -> &'static str {
        match self.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
        }
    }
}

/// `terramantle doctor`.
pub fn run(cli: &Cli, args: &DoctorArgs) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest_path = root.join(Manifest::FILENAME);
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "error: no {} here — run `terramantle init` first.",
                Manifest::FILENAME
            );
            return Ok(EXIT_FINDINGS);
        }
    };
    let manifest = match Manifest::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_FINDINGS);
        }
    };

    let mut checks = vec![check_scaffold(&root, &manifest)];
    checks.extend(check_terramate(&root, &manifest));
    checks.extend(check_lock_files(&root, &manifest));
    checks.extend(check_backend_workspaces(&root, &manifest));
    checks.push(check_trust(cli, &manifest, &root, args.offline));

    let mut failed = 0usize;
    for c in &checks {
        println!("  [{}] {:<22} {}", c.glyph(), c.name, c.detail);
        if c.status == Status::Fail {
            failed += 1;
        }
    }
    if failed == 0 {
        eprintln!("\nNo problems found.");
        Ok(0)
    } else {
        eprintln!("\n{failed} problem(s) found.");
        Ok(EXIT_FINDINGS)
    }
}

/// Is the committed scaffold what the manifest currently describes?
fn check_scaffold(root: &Path, m: &Manifest) -> Check {
    let populated = tm_release::artefacts(root, m)
        .map(|a| {
            a.iter()
                .any(|art| art.rel_dir != "." && root.join(&art.rel_dir).is_dir())
        })
        .unwrap_or(false);
    let desired = desired_files_with(m, !populated);
    let plan = compute(&desired, |rel: &Path| {
        std::fs::read_to_string(root.join(rel)).ok()
    });
    if plan.is_empty_of_changes() {
        Check::ok("scaffold", "up to date with terramantle.hcl")
    } else {
        let (a, u, p) = plan.counts();
        Check::fail(
            "scaffold",
            format!(
                "drifted ({a} to add, {u} to update, {p} to remove) — run `terramantle upgrade`"
            ),
        )
    }
}

/// Terramate's generated backends must be committed and current, because CI
/// gates on exactly this and would otherwise fail there instead of here.
fn check_terramate(root: &Path, m: &Manifest) -> Vec<Check> {
    if m.terramate.is_none() {
        return vec![];
    }
    if !on_path("terramate") {
        return vec![Check::skip(
            "terramate generate",
            "terramate not on PATH (https://terramate.io/docs/cli/installation)",
        )];
    }
    let out = std::process::Command::new("terramate")
        .arg("--chdir")
        .arg(root)
        .args(["generate", "--detailed-exit-code"])
        .output();
    match out {
        // 2 means it had to change something, i.e. the commit is missing a run.
        Ok(o) if o.status.code() == Some(2) => vec![Check::fail(
            "terramate generate",
            "generated code is stale — run `terramate generate` and commit the result",
        )],
        Ok(o) if o.status.success() => {
            vec![Check::ok(
                "terramate generate",
                "generated code is up to date",
            )]
        }
        Ok(o) => vec![Check::fail(
            "terramate generate",
            format!(
                "terramate failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        )],
        Err(e) => vec![Check::skip(
            "terramate generate",
            format!("could not run: {e}"),
        )],
    }
}

/// `state publish` skips any workspace without a committed lock file.
fn check_lock_files(root: &Path, m: &Manifest) -> Vec<Check> {
    if m.artefact != tm_scaffold::Artefact::Workspaces {
        return vec![];
    }
    let Ok(artefacts) = tm_release::artefacts(root, m) else {
        return vec![];
    };
    let missing: Vec<String> = artefacts
        .iter()
        .filter(|a| !root.join(&a.rel_dir).join(".terraform.lock.hcl").is_file())
        .map(|a| a.rel_dir.clone())
        .collect();
    if artefacts.is_empty() {
        return vec![Check::fail(
            "lock files",
            format!(
                "no workspaces matched discovery ({}) — state publish would find nothing",
                m.discovery
                    .as_ref()
                    .map(|d| d.include.join(", "))
                    .unwrap_or_else(|| ".".into())
            ),
        )];
    }
    if missing.is_empty() {
        vec![Check::ok(
            "lock files",
            format!(
                "all {} workspace(s) have .terraform.lock.hcl",
                artefacts.len()
            ),
        )]
    } else {
        vec![Check::fail(
            "lock files",
            format!(
                "{} workspace(s) missing .terraform.lock.hcl ({}) — state publish skips them",
                missing.len(),
                missing.join(", ")
            ),
        )]
    }
}

/// Every stack must declare a backend address, because that address is what
/// `state publish` now derives the workspace name from.
///
/// This used to assert the address matched the directory basename. It no
/// longer has to: publish reads the address itself, so a deliberately prefixed
/// workspace (`state-sample-<stack>`) is correct. What is *not* fine is a stack
/// with no readable address — publish then falls back to the directory name and
/// may push the lock file somewhere the state is not.
fn check_backend_workspaces(root: &Path, m: &Manifest) -> Vec<Check> {
    if m.artefact != tm_scaffold::Artefact::Workspaces {
        return vec![];
    }
    let Ok(artefacts) = tm_release::artefacts(root, m) else {
        return vec![];
    };
    if artefacts.is_empty() {
        return vec![];
    }

    let (mut resolved, mut unreadable) = (Vec::new(), Vec::new());
    for a in &artefacts {
        match tm_scaffold::workspace_from_backend(&root.join(&a.rel_dir)) {
            Some(ws) => resolved.push(ws),
            None => unreadable.push(a.rel_dir.clone()),
        }
    }

    let mut checks = vec![if unreadable.is_empty() {
        Check::ok(
            "backend workspaces",
            format!("all {} resolve ({})", resolved.len(), preview(&resolved)),
        )
    } else {
        Check::fail(
            "backend workspaces",
            format!(
                "{} stack(s) have no readable backend address ({}) — publish would fall back \
                 to the directory name",
                unreadable.len(),
                unreadable.join(", ")
            ),
        )
    }];

    // Two stacks pointing at one workspace is data loss: whichever applies
    // last overwrites the other's state, and nothing reports it.
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    for ws in &resolved {
        *seen.entry(ws.as_str()).or_default() += 1;
    }
    let dupes: Vec<&str> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(w, _)| *w)
        .collect();
    if !dupes.is_empty() {
        checks.push(Check::fail(
            "workspace collisions",
            format!(
                "{} workspace(s) are claimed by more than one stack ({}) — their state \
                 would overwrite each other",
                dupes.len(),
                dupes.join(", ")
            ),
        ));
    }
    checks
}

/// The first few names, so the line stays readable in a repo with many stacks.
fn preview(names: &[String]) -> String {
    const MAX: usize = 3;
    if names.len() <= MAX {
        return names.join(", ");
    }
    format!("{}, +{} more", names[..MAX].join(", "), names.len() - MAX)
}

/// Is this repo trusted for keyless CI auth?
fn check_trust(cli: &Cli, m: &Manifest, root: &Path, offline: bool) -> Check {
    if m.ci.auth != tm_scaffold::CiAuth::Oidc {
        return Check::skip("vcs trust", "auth=bot does not use VCS trust");
    }
    if offline {
        return Check::skip("vcs trust", "--offline");
    }
    let Some(remote) = tm_scaffold::vcs::origin_remote(root) else {
        return Check::warn("vcs trust", "no `origin` remote to derive a subject from");
    };
    let Some(subject) = crate::trust::subject_for(&remote, m.vcs) else {
        return Check::warn(
            "vcs trust",
            format!("cannot derive a subject from {remote}"),
        );
    };
    let Some(client) = crate::discovery::client_for_wizard(cli) else {
        return Check::skip(
            "vcs trust",
            "not authenticated — run `terramantle auth login`",
        );
    };
    let provider = match m.vcs {
        VcsProvider::Github => "github",
        VcsProvider::Gitlab => "gitlab",
    };
    match client.vcs_trust_list(&m.org) {
        Ok(entries) => {
            if entries
                .iter()
                .any(|e| e.subject == subject && e.provider == provider)
            {
                Check::ok("vcs trust", format!("{subject} is trusted"))
            } else {
                Check::fail(
                    "vcs trust",
                    format!(
                        "{subject} is NOT trusted — run `terramantle trust add`, or CI will 401"
                    ),
                )
            }
        }
        // Listing is admin-only, so a refusal is normal and not a finding.
        Err(e) => Check::skip("vcs trust", format!("could not read the trust table: {e}")),
    }
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_truncates_a_long_list() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(preview(&names), "a, b, c, +2 more");
        assert_eq!(preview(&names[..2]), "a, b");
    }
}
