//! VCS trust: teaching the org to accept this repo's CI tokens.
//!
//! Keyless CI auth works by the registry matching the `sub` claim of a
//! GitHub/GitLab OIDC token against a trust entry on the org. Without that row
//! every call from the pipeline 401s — and nothing in the scaffolded repo hints
//! at why, because the repo is correct; the missing state is server-side.
//!
//! `init` knows the org and the `origin` remote, which is everything needed to
//! compute the subject, so it offers to register it rather than leaving the
//! user to discover the problem from a red pipeline.

use tm_api::Client;
use tm_scaffold::{Manifest, VcsProvider};

use crate::commands::CmdResult;

/// The trust subject CI will present.
///
/// GitHub mints `repo:<owner>/<repo>:<more claims>`; GitLab mints
/// `project_path:<group>/<project>:<more claims>`. The registry matches the
/// documented prefix form against the full `sub`, so the stored subject is just
/// the first two segments.
pub fn subject_for(remote_url: &str, vcs: VcsProvider) -> Option<String> {
    let path = repo_path(remote_url)?;
    Some(match vcs {
        VcsProvider::Github => format!("repo:{path}"),
        VcsProvider::Gitlab => format!("project_path:{path}"),
    })
}

/// `owner/repo` (or `group/subgroup/project`) from any git remote form.
fn repo_path(remote_url: &str) -> Option<String> {
    let s = remote_url.trim();
    let after_host = if let Some(rest) = s.strip_prefix("git@") {
        // scp-like: git@host:owner/repo.git
        rest.split_once(':').map(|(_, p)| p)?
    } else {
        let mut rest = s;
        for scheme in ["ssh://", "https://", "http://"] {
            if let Some(r) = s.strip_prefix(scheme) {
                rest = r;
                break;
            }
        }
        if rest == s && !s.contains('/') {
            return None;
        }
        // Drop credentials, then host (and any :port).
        let no_creds = rest.rsplit('@').next().unwrap_or(rest);
        no_creds.split_once('/').map(|(_, p)| p)?
    };
    let path = after_host
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or_else(|| after_host.trim_matches('/'));
    // Two segments minimum, or it is not a repo path.
    if path.split('/').filter(|s| !s.is_empty()).count() < 2 {
        return None;
    }
    Some(path.to_string())
}

/// Offer to register this repo's trust subject, or explain how to do it.
///
/// Best-effort and never fatal: the scaffold is already written by the time
/// this runs, and a missing permission or an offline registry must not turn a
/// successful `init` into a failure.
pub fn offer(client: &Client, manifest: &Manifest, root: &std::path::Path, assume_yes: bool) {
    let Some(remote) = tm_scaffold::vcs::origin_remote(root) else {
        return;
    };
    let Some(subject) = subject_for(&remote, manifest.vcs) else {
        return;
    };
    let provider = match manifest.vcs {
        VcsProvider::Github => "github",
        VcsProvider::Gitlab => "gitlab",
    };

    // Already trusted? Then say nothing: this is the common re-run case.
    match client.vcs_trust_list(&manifest.org) {
        Ok(entries) => {
            if entries
                .iter()
                .any(|e| e.subject == subject && e.provider == provider)
            {
                eprintln!("  · VCS trust already registered for {subject}");
                return;
            }
        }
        Err(_) => {
            // Listing needs org-admin. Not being one is normal, so fall through
            // to the instructions rather than reporting a failure.
            explain(&manifest.org, provider, &subject);
            return;
        }
    }

    let proceed = if assume_yes {
        true
    } else if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        matches!(
            inquire::Confirm::new(&format!("Trust this repo ({subject}) for CI auth?"))
                .with_default(true)
                .with_help_message(
                    "without this, the first pipeline run gets 401 from the registry"
                )
                .prompt(),
            Ok(true)
        )
    } else {
        false
    };
    if !proceed {
        explain(&manifest.org, provider, &subject);
        return;
    }

    match client.vcs_trust_create(&manifest.org, provider, &subject) {
        Ok(()) => eprintln!("  + VCS trust registered for {subject}"),
        Err(e) => {
            eprintln!("  ! could not register VCS trust: {e}");
            explain(&manifest.org, provider, &subject);
        }
    }
}

fn explain(org: &str, provider: &str, subject: &str) {
    eprintln!(
        "  · CI auth needs this repo trusted by the org. Ask an org admin to run:\n      \
         terramantle trust add --provider {provider} --subject {subject}\n    \
         or add it in the portal under {org} → Settings → VCS Trust."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_from_every_remote_form() {
        let cases = [
            "git@github.com:acme/repo.git",
            "https://github.com/acme/repo.git",
            "https://github.com/acme/repo",
            "ssh://git@github.com:22/acme/repo.git",
            "https://x-token@github.com/acme/repo.git",
        ];
        for c in cases {
            assert_eq!(
                subject_for(c, VcsProvider::Github).as_deref(),
                Some("repo:acme/repo"),
                "{c}"
            );
        }
    }

    /// GitLab subgroups are part of the path, so the subject keeps all of it.
    #[test]
    fn gitlab_keeps_nested_groups() {
        assert_eq!(
            subject_for("git@gitlab.com:group/sub/proj.git", VcsProvider::Gitlab).as_deref(),
            Some("project_path:group/sub/proj")
        );
    }

    #[test]
    fn non_repo_remotes_yield_nothing() {
        for c in ["", "not a url", "git@github.com:", "https://github.com/"] {
            assert_eq!(subject_for(c, VcsProvider::Github), None, "{c}");
        }
    }
}

// ---- `terramantle trust` --------------------------------------------------

/// `terramantle trust …`.
pub fn dispatch(command: &crate::cli::TrustCommand, cli: &crate::cli::Cli) -> CmdResult {
    match command {
        crate::cli::TrustCommand::Ls => ls(cli),
        crate::cli::TrustCommand::Add { subject, provider } => {
            add(cli, subject.as_deref(), *provider)
        }
    }
}

fn ls(cli: &crate::cli::Cli) -> CmdResult {
    let (client, org) = match crate::discovery::client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let entries = match client.vcs_trust_list(&org) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(e.exit_code());
        }
    };
    if entries.is_empty() {
        eprintln!("no trusted subjects in org '{org}'");
    }
    let rows: Vec<TrustRow> = entries
        .into_iter()
        .map(|e| TrustRow {
            provider: e.provider,
            subject: e.subject,
        })
        .collect();
    if crate::output::print_structured(&rows, cli.global.output.unwrap_or_default())? {
        return Ok(0);
    }
    let mut table = crate::output::TableView::new(["provider", "subject"]);
    for r in &rows {
        table.row([r.provider.clone(), r.subject.clone()]);
    }
    println!("{}", table.render());
    Ok(0)
}

#[derive(serde::Serialize)]
struct TrustRow {
    provider: String,
    subject: String,
}

/// Trust a repo. The subject defaults to this repo's `origin` remote, which is
/// the case that matters: the thing a user needs trusted is almost always the
/// repo they are standing in.
fn add(cli: &crate::cli::Cli, subject: Option<&str>, provider: Option<VcsProvider>) -> CmdResult {
    let root = std::env::current_dir()?;
    let remote = tm_scaffold::vcs::origin_remote(&root);

    let provider = match provider.or_else(|| {
        remote
            .as_deref()
            .and_then(tm_scaffold::vcs::infer_from_remote)
    }) {
        Some(p) => p,
        None => {
            eprintln!("error: could not infer the provider; pass --provider github|gitlab");
            return Ok(EXIT_USAGE);
        }
    };

    let subject = match subject
        .map(str::to_string)
        .or_else(|| remote.as_deref().and_then(|r| subject_for(r, provider)))
    {
        Some(s) => s,
        None => {
            eprintln!("error: could not derive a subject from the `origin` remote; pass --subject");
            return Ok(EXIT_USAGE);
        }
    };

    let (client, org) = match crate::discovery::client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let provider_str = match provider {
        VcsProvider::Github => "github",
        VcsProvider::Gitlab => "gitlab",
    };
    match client.vcs_trust_create(&org, provider_str, &subject) {
        Ok(()) => {
            eprintln!("trusted {subject} ({provider_str}) in org '{org}'");
            Ok(0)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Ok(e.exit_code())
        }
    }
}

/// Exit 2: a usage error (no subject, no provider).
const EXIT_USAGE: i32 = 2;
