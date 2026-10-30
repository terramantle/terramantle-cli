//! VCS provider inference from the `origin` remote (SCAFFOLD-PUBLISH-AUTH.md §2).
//!
//! `github.com` → GitHub → `.github/workflows/terramantle.yml`.
//! `gitlab.*`   → GitLab → `.gitlab-ci.yml`.
//! anything else / no remote → undetermined (the CLI prompts, or errors headless).

use std::path::Path;
use std::process::Command;

use crate::error::ScaffoldError;
use crate::manifest::VcsProvider;

/// Infer the provider from a remote URL. Pure — the git call is separated into
/// [`origin_remote`] so this is exhaustively unit-testable.
///
/// Handles both SSH (`git@github.com:org/repo.git`) and HTTPS
/// (`https://gitlab.example.com/org/repo.git`) forms, and self-managed GitLab
/// hosts whose hostname contains `gitlab`.
pub fn infer_from_remote(remote_url: &str) -> Option<VcsProvider> {
    let host = host_of(remote_url)?.to_ascii_lowercase();
    if host == "github.com" || host.ends_with(".github.com") || host.contains("github") {
        Some(VcsProvider::Github)
    } else if host.contains("gitlab") {
        Some(VcsProvider::Gitlab)
    } else {
        None
    }
}

/// Extract the host from an SSH-style or URL-style git remote.
fn host_of(remote_url: &str) -> Option<&str> {
    let s = remote_url.trim();
    // scp-like syntax: [user@]host:path
    if let Some(rest) = s.strip_prefix("git@") {
        return rest.split(':').next().filter(|h| !h.is_empty());
    }
    // ssh://git@host/… or https://host/… or http://host/…
    for scheme in ["ssh://", "https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            let after_creds = rest.rsplit('@').next().unwrap_or(rest);
            let host = after_creds
                .split(['/', ':'])
                .next()
                .filter(|h| !h.is_empty())?;
            return Some(host);
        }
    }
    // Fallback: bare `host:path` (no user@).
    if s.contains(':') && !s.contains("://") {
        return s.split(':').next().filter(|h| !h.is_empty());
    }
    None
}

/// Rewrite a git remote as a URL that can be sent as `X-Git-Repo-URL` and
/// followed in a browser.
///
/// `git remote get-url` returns whatever the clone used, and the usual SSH form
/// is scp syntax (`git@github.com:acme/repo.git`), which has no scheme at all.
/// The state backend validates the header — the scheme must be `https`, `http`
/// or `ssh` — so sending that verbatim is a `422` on every push, and the portal
/// only renders an `https` remote as a link. An SSH or `git://` remote names a
/// transport a browser cannot follow, so it is rebuilt over https without the
/// credentials, the SSH port or the `.git` suffix, none of which mean anything
/// there. An `http(s)` remote already IS the web URL and is passed through
/// untouched.
///
/// `None` means there was no host to build a URL from — a local-path clone, say.
/// Attribution is optional, so the caller drops the header rather than failing
/// the upload over it.
pub fn web_remote_url(remote_url: &str) -> Option<String> {
    let s = remote_url.trim();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            return (!rest.is_empty()).then(|| s.to_string());
        }
    }
    let (host, path) = ssh_host_and_path(s)?;
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

/// Split an SSH-style remote into its host and repo path. Separate from
/// [`host_of`] because the rewrite needs the path too, and because the port in
/// `ssh://host:2222/…` is a port while the colon in `host:acme/repo` is not.
fn ssh_host_and_path(s: &str) -> Option<(&str, &str)> {
    for scheme in ["ssh://", "git://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            let (authority, path) = rest.split_once('/')?;
            let authority = authority.rsplit('@').next().unwrap_or(authority);
            let host = authority.split(':').next().filter(|h| !h.is_empty())?;
            return Some((host, path));
        }
    }
    if s.contains("://") {
        return None; // file://, or a scheme we have no web form for.
    }
    // scp syntax: [user@]host:path, where the colon separates the two.
    let (authority, path) = s.split_once(':')?;
    let host = authority.rsplit('@').next().unwrap_or(authority);
    // A bare local path such as `/srv/git/repo` has no colon and never reaches
    // here; `/srv/a:b` would, so reject anything with a slash in the host.
    if host.is_empty() || host.contains('/') {
        return None;
    }
    Some((host, path))
}

/// The `origin` remote URL for the repo containing `dir`, or `None` if there is
/// no remote / not a git repo.
pub fn origin_remote(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// True if `dir` is inside a git working tree.
pub fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve the provider for `dir`: explicit override wins; otherwise infer from
/// the `origin` remote; otherwise error (the CLI turns this into a wizard prompt
/// on a TTY).
pub fn resolve(dir: &Path, explicit: Option<VcsProvider>) -> Result<VcsProvider, ScaffoldError> {
    if let Some(v) = explicit {
        return Ok(v);
    }
    origin_remote(dir)
        .as_deref()
        .and_then(infer_from_remote)
        .ok_or(ScaffoldError::VcsUndetermined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_github_ssh() {
        assert_eq!(
            infer_from_remote("git@github.com:acme/repo.git"),
            Some(VcsProvider::Github)
        );
    }

    #[test]
    fn infers_github_https() {
        assert_eq!(
            infer_from_remote("https://github.com/acme/repo.git"),
            Some(VcsProvider::Github)
        );
    }

    #[test]
    fn infers_gitlab_saas_and_self_managed() {
        assert_eq!(
            infer_from_remote("git@gitlab.com:acme/repo.git"),
            Some(VcsProvider::Gitlab)
        );
        assert_eq!(
            infer_from_remote("https://gitlab.internal.acme.io/acme/repo.git"),
            Some(VcsProvider::Gitlab)
        );
        assert_eq!(
            infer_from_remote("ssh://git@gitlab.example.com:2222/acme/repo.git"),
            Some(VcsProvider::Gitlab)
        );
    }

    #[test]
    fn unknown_host_is_undetermined() {
        assert_eq!(infer_from_remote("git@bitbucket.org:acme/repo.git"), None);
        assert_eq!(infer_from_remote(""), None);
        assert_eq!(infer_from_remote("not a url"), None);
    }

    #[test]
    fn host_parsing_covers_forms() {
        assert_eq!(host_of("git@github.com:a/b.git"), Some("github.com"));
        assert_eq!(host_of("https://x@gitlab.com/a/b"), Some("gitlab.com"));
        assert_eq!(host_of("ssh://git@host.io:22/a/b"), Some("host.io"));
    }

    #[test]
    fn scp_syntax_becomes_an_https_url() {
        // The form `git clone git@github.com:…` leaves behind, and the one the
        // state backend used to 422 on because it carries no scheme.
        assert_eq!(
            web_remote_url("git@github.com:acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web_remote_url("git@github.com:acme/repo").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web_remote_url("github.com:acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
    }

    #[test]
    fn ssh_and_git_schemes_lose_the_credentials_and_port() {
        assert_eq!(
            web_remote_url("ssh://git@gitlab.example.com:2222/acme/repo.git").as_deref(),
            Some("https://gitlab.example.com/acme/repo")
        );
        assert_eq!(
            web_remote_url("git://github.com/acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
    }

    #[test]
    fn http_remotes_pass_through_untouched() {
        // Already the web URL: rewriting one the user set deliberately would be
        // a surprise, so even the `.git` suffix stays.
        for url in [
            "https://github.com/acme/repo",
            "https://github.com/acme/repo.git",
            "http://git.internal/acme/repo",
            "https://x-token@github.com/acme/repo.git",
        ] {
            assert_eq!(web_remote_url(url).as_deref(), Some(url));
        }
    }

    #[test]
    fn remotes_with_no_web_form_are_dropped() {
        for url in [
            "",
            "   ",
            "/srv/git/repo.git",
            "../sibling-repo",
            "file:///srv/git/repo.git",
            "git@github.com:",
            "https://",
        ] {
            assert_eq!(web_remote_url(url), None, "expected no web URL for {url:?}");
        }
    }

    #[test]
    fn explicit_override_wins_without_touching_git() {
        // A non-existent dir would fail inference, but the explicit value wins.
        let dir = Path::new("/nonexistent-xyz");
        assert_eq!(
            resolve(dir, Some(VcsProvider::Gitlab)).unwrap(),
            VcsProvider::Gitlab
        );
    }
}
