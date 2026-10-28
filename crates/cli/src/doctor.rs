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

/// The backend's workspace segment must equal the directory basename, because
/// that is what `state publish` derives the workspace name from. When they
/// disagree the lock file and the state land in different workspaces, and
/// nothing errors — each call is individually valid.
fn check_backend_workspaces(root: &Path, m: &Manifest) -> Vec<Check> {
    if m.artefact != tm_scaffold::Artefact::Workspaces {
        return vec![];
    }
    let Ok(artefacts) = tm_release::artefacts(root, m) else {
        return vec![];
    };
    let mut mismatched = Vec::new();
    for a in &artefacts {
        let dir = root.join(&a.rel_dir);
        let Some(addr) = backend_address(&dir) else {
            continue;
        };
        let Some(ws) = addr.rsplit('/').next() else {
            continue;
        };
        if ws != a.name {
            mismatched.push(format!("{} → {ws}", a.rel_dir));
        }
    }
    if mismatched.is_empty() {
        vec![Check::ok(
            "backend workspaces",
            "every backend address matches its directory name",
        )]
    } else {
        vec![Check::fail(
            "backend workspaces",
            format!(
                "address does not match the directory basename ({}) — \
                 the lock file and the state would use different workspaces",
                mismatched.join(", ")
            ),
        )]
    }
}

/// The `address = "…/state/<org>/<workspace>"` of whichever backend file the
/// directory has. Text scan rather than an HCL parse: `_backend.tf` is machine
/// generated and single-purpose, and a parse failure here must not be fatal.
fn backend_address(dir: &Path) -> Option<String> {
    for name in ["_backend.tf", "backend.tf"] {
        let Ok(text) = std::fs::read_to_string(dir.join(name)) else {
            continue;
        };
        for line in text.lines() {
            let t = line.trim();
            if !t.starts_with("address") {
                continue;
            }
            if let Some(v) = t.split('=').nth(1) {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
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
    fn backend_address_is_read_from_either_filename() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("_backend.tf"),
            "terraform {\n  backend \"http\" {\n    \
             address        = \"https://r.example/state/acme/network\"\n    \
             lock_address   = \"https://r.example/state/acme/network.lock\"\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(
            backend_address(tmp.path()).as_deref(),
            Some("https://r.example/state/acme/network")
        );
    }

    /// `lock_address` also starts with "lock_address" — the scan must not
    /// return it for `address`.
    #[test]
    fn backend_address_ignores_the_lock_addresses() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("backend.tf"),
            "    lock_address   = \"https://r.example/state/acme/x.lock\"\n    \
             address        = \"https://r.example/state/acme/x\"\n",
        )
        .unwrap();
        assert_eq!(
            backend_address(tmp.path()).as_deref(),
            Some("https://r.example/state/acme/x")
        );
    }

    #[test]
    fn no_backend_file_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(backend_address(tmp.path()), None);
    }
}
