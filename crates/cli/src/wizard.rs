//! The `terramantle init` wizard.
//!
//! v1 `init` was a flag parser with two fallback prompts, and every input it
//! could not infer was a dead end: outside a git repo it printed "must be run
//! from inside a git repository" and stopped; with no org configured it printed
//! "pass --org, set TERRAMANTLE_ORG, or select a context" and stopped. Both are
//! answerable questions, so the wizard asks them.
//!
//! Three rules hold throughout:
//!
//! * **A flag always wins.** Every step checks its flag first and only prompts
//!   when the answer is genuinely unknown.
//! * **Non-interactive behaves like `--yes`.** Without a TTY there is nobody to
//!   ask, so the wizard takes defaults where it safely can and errors with the
//!   flag to pass where it cannot. CI must never block on a prompt.
//! * **The wizard cannot produce a repo that fails on its first push.** Where a
//!   combination is known-broken — OIDC against a self-managed GitLab, which the
//!   worker will never trust — it says so and picks the working option.

use std::io::IsTerminal;

use tm_api::Client;
use tm_scaffold::{
    vcs, Artefact, ChangeBase, CiAuth, CiConfig, Discovery, Manifest, ScaffoldError, Structure,
    Terramate, VcsProvider,
};

use crate::cli::{Cli, InitArgs};

/// Whether the wizard may ask anything at all.
///
/// Both streams matter: `inquire` draws on stderr and reads stdin, so a piped
/// stdin with a terminal stderr would hang waiting for input that never comes.
pub fn interactive(args: &InitArgs) -> bool {
    !args.yes && std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// A cancelled prompt (Esc / Ctrl-C) is a decision to stop, not an error to
/// wrap — it maps to the standard "confirmation refused" exit.
fn cancelled(e: &inquire::InquireError) -> bool {
    matches!(
        e,
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted
    )
}

/// What a prompt helper returns: an answer, or "the user stopped".
pub enum Answer<T> {
    Got(T),
    Cancelled,
}

fn select<T: std::fmt::Display>(
    question: &str,
    options: Vec<T>,
    help: Option<&str>,
) -> Result<Answer<T>, ScaffoldError> {
    let mut p = inquire::Select::new(question, options);
    if let Some(h) = help {
        p = p.with_help_message(h);
    }
    match p.prompt() {
        Ok(v) => Ok(Answer::Got(v)),
        Err(e) if cancelled(&e) => Ok(Answer::Cancelled),
        Err(e) => Err(ScaffoldError::Prompt(e.to_string())),
    }
}

fn confirm(
    question: &str,
    default: bool,
    help: Option<&str>,
) -> Result<Answer<bool>, ScaffoldError> {
    let mut p = inquire::Confirm::new(question).with_default(default);
    if let Some(h) = help {
        p = p.with_help_message(h);
    }
    match p.prompt() {
        Ok(v) => Ok(Answer::Got(v)),
        Err(e) if cancelled(&e) => Ok(Answer::Cancelled),
        Err(e) => Err(ScaffoldError::Prompt(e.to_string())),
    }
}

fn text(question: &str, default: &str) -> Result<Answer<String>, ScaffoldError> {
    match inquire::Text::new(question).with_default(default).prompt() {
        Ok(v) => Ok(Answer::Got(v)),
        Err(e) if cancelled(&e) => Ok(Answer::Cancelled),
        Err(e) => Err(ScaffoldError::Prompt(e.to_string())),
    }
}

// ---- individual steps -----------------------------------------------------

/// A labelled choice, so the menu can explain itself without the label becoming
/// the parsed value.
struct Choice<T> {
    value: T,
    label: &'static str,
}

impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

/// Step 1 — the repo must be a git repo, because every later step reads git:
/// the remote determines the VCS provider and the trust subject, and change
/// detection diffs against history.
pub fn ensure_git_repo(root: &std::path::Path, args: &InitArgs) -> Result<(), ScaffoldError> {
    if vcs::is_git_repo(root) {
        return Ok(());
    }
    if !interactive(args) {
        return Err(ScaffoldError::NotAGitRepo);
    }
    match confirm(
        "Not a git repository. Run `git init` here?",
        true,
        Some("terramantle needs git for change detection and to identify the repo to your org"),
    )? {
        Answer::Got(true) => {}
        _ => return Err(ScaffoldError::NotAGitRepo),
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("init")
        .output()
        .map_err(|e| ScaffoldError::Prompt(format!("could not run git init: {e}")))?;
    if !out.status.success() {
        return Err(ScaffoldError::Prompt(format!(
            "git init failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    eprintln!("  + initialised an empty git repository");
    Ok(())
}

/// Step 2 — the org. Config precedence first; then, for a human token, the
/// membership list. Reuses the same listing the rest of the CLI resolves orgs
/// from, so the wizard cannot offer an org that other commands would reject.
pub fn resolve_org<'a>(
    configured: Option<String>,
    client: &mut dyn FnMut() -> Option<&'a Client>,
    args: &InitArgs,
) -> Result<String, ScaffoldError> {
    // Config precedence first, so a configured org never triggers a network call.
    if let Some(org) = configured {
        return Ok(org);
    }
    let Some(client) = client() else {
        return Err(ScaffoldError::MissingOrg);
    };
    let memberships = client.orgs_list().map_err(|_| ScaffoldError::MissingOrg)?;
    match memberships.len() {
        0 => Err(ScaffoldError::MissingOrg),
        1 => {
            let org = memberships[0].slug.clone();
            eprintln!("  · org '{org}' (your only membership)");
            Ok(org)
        }
        _ if !interactive(args) => Err(ScaffoldError::MissingOrg),
        _ => {
            let options: Vec<String> = memberships.into_iter().map(|m| m.slug).collect();
            match select("Which org owns this repo?", options, None)? {
                Answer::Got(v) => Ok(v),
                Answer::Cancelled => Err(ScaffoldError::Cancelled),
            }
        }
    }
}

/// What the directory layout says this repo is, when it says anything.
///
/// Only an unambiguous answer counts: a repo with both `stacks/` and `modules/`
/// is a genuine question, and guessing one would scaffold the wrong pipeline.
pub fn detect_artefact(root: &std::path::Path) -> Option<Artefact> {
    let stacks = root.join("stacks").is_dir() || root.join("terramate.tm.hcl").is_file();
    let modules = root.join("modules").is_dir();
    match (stacks, modules) {
        (true, false) => Some(Artefact::Workspaces),
        (false, true) => Some(Artefact::Modules),
        _ => None,
    }
}

/// Step 3 — what the repo produces. Inferred from the layout when that is
/// unambiguous, so the common case needs no flag even headless.
pub fn resolve_artefact(
    root: &std::path::Path,
    args: &InitArgs,
) -> Result<Artefact, ScaffoldError> {
    if let Some(a) = args.artefact {
        return Ok(a);
    }
    if let Some(a) = detect_artefact(root) {
        eprintln!("  · artefact '{a}' (from the directory layout)");
        return Ok(a);
    }
    if !interactive(args) {
        return Err(ScaffoldError::BadEnum {
            field: "artefact",
            value: "<missing>".into(),
            expected: "modules workspaces",
        });
    }
    let options = vec![
        Choice {
            value: Artefact::Workspaces,
            label: "workspaces — Terraform state this repo applies (stacks, environments)",
        },
        Choice {
            value: Artefact::Modules,
            label: "modules — reusable modules this repo publishes to the registry",
        },
    ];
    match select("What does this repo contain?", options, None)? {
        Answer::Got(c) => Ok(c.value),
        Answer::Cancelled => Err(ScaffoldError::Cancelled),
    }
}

/// Step 4 — layout. Defaults to `mono` when the conventional directory already
/// exists, because that is a strong signal and re-asking it is noise.
pub fn resolve_structure(
    root: &std::path::Path,
    artefact: Artefact,
    args: &InitArgs,
) -> Result<Structure, ScaffoldError> {
    if let Some(s) = args.structure {
        return Ok(s);
    }
    let conventional = match artefact {
        Artefact::Modules => "modules",
        Artefact::Workspaces => "stacks",
    };
    let looks_mono = root.join(conventional).is_dir();
    if !interactive(args) {
        return Ok(if looks_mono {
            Structure::Mono
        } else {
            Structure::Poly
        });
    }
    let options = if looks_mono {
        vec![
            Choice {
                value: Structure::Mono,
                label: "mono — many independently versioned artefacts in this repo",
            },
            Choice {
                value: Structure::Poly,
                label: "poly — exactly one artefact, at the repo root",
            },
        ]
    } else {
        vec![
            Choice {
                value: Structure::Poly,
                label: "poly — exactly one artefact, at the repo root",
            },
            Choice {
                value: Structure::Mono,
                label: "mono — many independently versioned artefacts in this repo",
            },
        ]
    };
    let help = looks_mono.then_some("a `{conventional}/` directory already exists here");
    match select("Repo layout?", options, help)? {
        Answer::Got(c) => Ok(c.value),
        Answer::Cancelled => Err(ScaffoldError::Cancelled),
    }
}

/// The outcome of resolving the VCS provider: the provider, plus whether OIDC
/// is usable against it.
pub struct VcsChoice {
    pub provider: VcsProvider,
    /// True when the host is a self-managed GitLab. The worker trusts only the
    /// literal issuers `token.actions.githubusercontent.com` and `gitlab.com`,
    /// so an OIDC token from anywhere else can never validate.
    pub oidc_unsupported: bool,
}

/// Is this remote a GitLab the registry will never trust?
pub fn is_self_managed_gitlab(remote: &str, provider: VcsProvider) -> bool {
    if provider != VcsProvider::Gitlab {
        return false;
    }
    match host_of(remote) {
        Some(h) => {
            let h = h.to_ascii_lowercase();
            h != "gitlab.com" && !h.ends_with(".gitlab.com")
        }
        None => false,
    }
}

/// Host of an scp-style or URL-style git remote. Mirrors `tm_scaffold::vcs`,
/// which keeps its own copy private.
fn host_of(remote_url: &str) -> Option<&str> {
    let s = remote_url.trim();
    if let Some(rest) = s.strip_prefix("git@") {
        return rest.split(':').next().filter(|h| !h.is_empty());
    }
    for scheme in ["ssh://", "https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            let after_creds = rest.rsplit('@').next().unwrap_or(rest);
            return after_creds
                .split(['/', ':'])
                .next()
                .filter(|h| !h.is_empty());
        }
    }
    if s.contains(':') && !s.contains("://") {
        return s.split(':').next().filter(|h| !h.is_empty());
    }
    None
}

/// Step 5 — VCS provider, from the flag, else the `origin` remote, else asked.
pub fn resolve_vcs(root: &std::path::Path, args: &InitArgs) -> Result<VcsChoice, ScaffoldError> {
    let remote = vcs::origin_remote(root);
    let provider = match vcs::resolve(root, args.vcs) {
        Ok(v) => v,
        Err(ScaffoldError::VcsUndetermined) if interactive(args) => {
            let options = vec![
                Choice {
                    value: VcsProvider::Github,
                    label: "github",
                },
                Choice {
                    value: VcsProvider::Gitlab,
                    label: "gitlab",
                },
            ];
            match select(
                "Which CI should be scaffolded?",
                options,
                Some("could not infer it from the `origin` remote"),
            )? {
                Answer::Got(c) => c.value,
                Answer::Cancelled => return Err(ScaffoldError::Cancelled),
            }
        }
        Err(e) => return Err(e),
    };
    let oidc_unsupported = remote
        .as_deref()
        .map(|r| is_self_managed_gitlab(r, provider))
        .unwrap_or(false);
    Ok(VcsChoice {
        provider,
        oidc_unsupported,
    })
}

/// Step 6 — CI auth. A self-managed GitLab is forced onto `bot`: keyless OIDC
/// there is not "discouraged", it cannot work, because the registry only
/// accepts the two literal SaaS issuers.
pub fn resolve_ci_auth(choice: &VcsChoice, args: &InitArgs) -> CiAuth {
    if let Some(a) = args.ci_auth {
        if a == CiAuth::Oidc && choice.oidc_unsupported {
            eprintln!(
                "  ! --ci-auth oidc ignored: this is a self-managed GitLab, and the registry\n    \
                 trusts only gitlab.com as an OIDC issuer. Using auth=bot instead."
            );
            return CiAuth::Bot;
        }
        return a;
    }
    if choice.oidc_unsupported {
        eprintln!(
            "  ! self-managed GitLab detected. Keyless OIDC needs the literal gitlab.com\n    \
             issuer, so this repo is scaffolded with auth=bot. Create a bot token in\n    \
             the portal under Settings → Tokens."
        );
        return CiAuth::Bot;
    }
    CiAuth::Oidc
}

/// Step 7 — Terramate. Workspaces only: a modules repo has no stacks to
/// orchestrate, and `terramantle modules changed` already does its change
/// detection.
pub fn resolve_terramate(
    root: &std::path::Path,
    artefact: Artefact,
    args: &InitArgs,
) -> Result<Option<Terramate>, ScaffoldError> {
    if artefact != Artefact::Workspaces {
        return Ok(None);
    }
    if args.no_terramate {
        return Ok(None);
    }
    let adopted = root.join("terramate.tm.hcl").exists();
    if args.terramate || adopted || !interactive(args) {
        if adopted {
            eprintln!("  · adopting the existing terramate.tm.hcl");
        }
        return Ok(Some(Terramate::default()));
    }
    let answer = confirm(
        "Orchestrate stacks with Terramate?",
        true,
        Some("generates each stack's backend and runs only the stacks a change touches"),
    )?;
    match answer {
        Answer::Got(true) => Ok(Some(Terramate::default())),
        Answer::Got(false) => Ok(None),
        Answer::Cancelled => Err(ScaffoldError::Cancelled),
    }
}

/// Step 8 — the trunk branch. Read from git rather than assumed, because the
/// apply job keys off it and guessing `main` on a `master` repo produces a
/// pipeline that never applies anything.
pub fn resolve_default_branch(
    root: &std::path::Path,
    args: &InitArgs,
) -> Result<String, ScaffoldError> {
    if let Some(b) = &args.default_branch {
        return Ok(b.clone());
    }
    let detected = detect_default_branch(root).unwrap_or_else(|| "main".to_string());
    if !interactive(args) {
        return Ok(detected);
    }
    match text("Default branch (apply runs here)?", &detected)? {
        Answer::Got(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        Answer::Got(_) => Ok(detected),
        Answer::Cancelled => Err(ScaffoldError::Cancelled),
    }
}

/// The repo's trunk: the current branch of a repo with commits, else the
/// configured `init.defaultBranch`, else None.
fn detect_default_branch(root: &std::path::Path) -> Option<String> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    // `origin/HEAD` is the authoritative answer when the remote has been cloned.
    if let Some(sym) = git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if let Some(name) = sym.strip_prefix("origin/") {
            return Some(name.to_string());
        }
    }
    git(&["symbolic-ref", "--short", "HEAD"]).or_else(|| git(&["config", "init.defaultBranch"]))
}

/// Step 9 — how `--changed` picks its base. The pipeline's own default
/// (merge-base against the target branch, `HEAD^` on the trunk) is already
/// correct for merge, squash and rebase merges, so this is an override for the
/// unusual repo rather than a correctness question.
pub fn resolve_change_base(args: &InitArgs) -> Result<ChangeBase, ScaffoldError> {
    if let Some(c) = args.change_base {
        return Ok(c);
    }
    Ok(ChangeBase::Auto)
}

// ---- assembling the manifest ---------------------------------------------

/// Build the `ci` block from the resolved answers and the feature flags.
#[allow(clippy::too_many_arguments)]
pub fn ci_config(
    args: &InitArgs,
    auth: CiAuth,
    default_branch: String,
    change_base: ChangeBase,
) -> CiConfig {
    let d = CiConfig::default();
    CiConfig {
        engine: d.engine,
        tofu_versions: args
            .tofu
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or(d.tofu_versions),
        auth,
        lint: !args.no_lint,
        security_scan: !args.no_scan,
        docs: !args.no_docs,
        versioning: args.versioning.unwrap_or(d.versioning),
        default_branch,
        runner: args.runner.clone().unwrap_or(d.runner),
        pr_comment: !args.no_pr_comment,
        change_base,
    }
}

/// Run every step and assemble the manifest.
pub fn build<'a>(
    cli: &Cli,
    args: &InitArgs,
    root: &std::path::Path,
    configured_org: Option<String>,
    api_url: String,
    client: &mut dyn FnMut() -> Option<&'a Client>,
) -> Result<Manifest, ScaffoldError> {
    let _ = cli;
    let org = resolve_org(configured_org, client, args)?;
    let artefact = resolve_artefact(root, args)?;
    let structure = resolve_structure(root, artefact, args)?;
    let vcs_choice = resolve_vcs(root, args)?;
    let auth = resolve_ci_auth(&vcs_choice, args);
    let terramate = resolve_terramate(root, artefact, args)?;
    let default_branch = resolve_default_branch(root, args)?;
    let change_base = resolve_change_base(args)?;

    Ok(Manifest {
        structure,
        artefact,
        org,
        // Only pin api_url when it diverges from the discovery default.
        api_url: (api_url != tm_config::DEFAULT_API_URL).then_some(api_url),
        vcs: vcs_choice.provider,
        discovery: match structure {
            Structure::Mono => Some(Discovery::default_for(artefact)),
            Structure::Poly => None,
        },
        terramate,
        ci: ci_config(args, auth, default_branch, change_base),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_managed_gitlab_is_detected_from_any_remote_form() {
        for remote in [
            "git@gitlab.internal.acme.io:team/repo.git",
            "https://gitlab.corp.example/team/repo.git",
            "ssh://git@gitlab.example.com:2222/team/repo.git",
        ] {
            assert!(
                is_self_managed_gitlab(remote, VcsProvider::Gitlab),
                "{remote} should be self-managed"
            );
        }
    }

    #[test]
    fn gitlab_dot_com_is_not_self_managed() {
        for remote in [
            "git@gitlab.com:acme/repo.git",
            "https://gitlab.com/acme/repo.git",
            "https://x@gitlab.com/acme/repo",
        ] {
            assert!(!is_self_managed_gitlab(remote, VcsProvider::Gitlab));
        }
    }

    /// GitHub is never "self-managed GitLab", whatever the host looks like.
    #[test]
    fn github_is_never_flagged() {
        assert!(!is_self_managed_gitlab(
            "git@github.enterprise.acme.io:a/b.git",
            VcsProvider::Github
        ));
    }

    /// OIDC against a self-managed GitLab can never validate, so the wizard
    /// must override it rather than scaffold a pipeline that always 401s.
    #[test]
    fn self_managed_gitlab_forces_bot_auth() {
        let choice = VcsChoice {
            provider: VcsProvider::Gitlab,
            oidc_unsupported: true,
        };
        let args = InitArgs {
            ci_auth: Some(CiAuth::Oidc),
            ..InitArgs::default()
        };
        assert_eq!(resolve_ci_auth(&choice, &args), CiAuth::Bot);

        // And with no flag at all.
        assert_eq!(resolve_ci_auth(&choice, &InitArgs::default()), CiAuth::Bot);
    }

    #[test]
    fn supported_hosts_default_to_oidc() {
        let choice = VcsChoice {
            provider: VcsProvider::Github,
            oidc_unsupported: false,
        };
        assert_eq!(resolve_ci_auth(&choice, &InitArgs::default()), CiAuth::Oidc);
        // An explicit bot choice is still honoured.
        let args = InitArgs {
            ci_auth: Some(CiAuth::Bot),
            ..InitArgs::default()
        };
        assert_eq!(resolve_ci_auth(&choice, &args), CiAuth::Bot);
    }

    /// An unambiguous layout answers the question; an ambiguous one must not
    /// be guessed, because guessing scaffolds the wrong pipeline entirely.
    #[test]
    fn artefact_detection_only_fires_when_unambiguous() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(detect_artefact(root), None, "empty repo");

        std::fs::create_dir(root.join("stacks")).unwrap();
        assert_eq!(detect_artefact(root), Some(Artefact::Workspaces));

        std::fs::create_dir(root.join("modules")).unwrap();
        assert_eq!(
            detect_artefact(root),
            None,
            "both present is a real question"
        );

        std::fs::remove_dir(root.join("stacks")).unwrap();
        assert_eq!(detect_artefact(root), Some(Artefact::Modules));
    }

    /// An existing terramate.tm.hcl is as strong a signal as a stacks/ dir.
    #[test]
    fn terramate_config_implies_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("terramate.tm.hcl"), "terramate {}\n").unwrap();
        assert_eq!(detect_artefact(tmp.path()), Some(Artefact::Workspaces));
    }

    #[test]
    fn ci_config_maps_the_negative_flags() {
        let args = InitArgs {
            no_lint: true,
            no_docs: true,
            no_pr_comment: true,
            ..InitArgs::default()
        };
        let ci = ci_config(&args, CiAuth::Oidc, "main".into(), ChangeBase::Auto);
        assert!(!ci.lint);
        assert!(!ci.docs);
        assert!(!ci.pr_comment);
        assert!(ci.security_scan, "not disabled, so it stays on");
    }

    #[test]
    fn tofu_versions_flag_overrides_the_pin() {
        let args = InitArgs {
            tofu: Some(vec!["1.12.0".into()]),
            ..InitArgs::default()
        };
        let ci = ci_config(&args, CiAuth::Oidc, "main".into(), ChangeBase::Auto);
        assert_eq!(ci.tofu_version(), "1.12.0");
    }
}
