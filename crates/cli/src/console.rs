//! `terramantle console` — open the human portal in the default browser.
//!
//! No authentication: the portal runs its own SPA login. The only network call
//! is bootstrap discovery, to learn the deployment's `portal_url`, and even that
//! is optional — a failure falls back to [`tm_config::DEFAULT_PORTAL_URL`] so the
//! command still works offline or against a worker that predates the field.

use std::process::Command;

use tm_auth::discovery;

use crate::cli::{Cli, ConsoleArgs};
use crate::commands::CmdResult;

/// Resolve the URL to open: `--url` > `TERRAMANTLE_PORTAL_URL` > discovery >
/// [`tm_config::DEFAULT_PORTAL_URL`], with `path` appended when given.
fn resolve_url(args: &ConsoleArgs, api_url: &str) -> String {
    let override_url = args
        .url
        .clone()
        .or_else(|| std::env::var("TERRAMANTLE_PORTAL_URL").ok())
        .filter(|s| !s.is_empty());

    // Discovery is best-effort: an unreachable registry must not stop us opening
    // the portal, so a fetch failure just leaves the accessor on its default.
    let base = match (&override_url, discovery::fetch(api_url)) {
        (Some(u), _) => u.clone(),
        (None, Ok(disco)) => disco.portal_url(None).to_string(),
        (None, Err(_)) => tm_config::DEFAULT_PORTAL_URL.to_string(),
    };

    let base = base.trim_end_matches('/');
    match args.path.as_deref().map(|p| p.trim_start_matches('/')) {
        Some(path) if !path.is_empty() => format!("{base}/{path}"),
        _ => base.to_string(),
    }
}

/// The platform's "open this in the user's default browser" command.
fn opener() -> (&'static str, &'static [&'static str]) {
    #[cfg(target_os = "macos")]
    return ("open", &[]);
    #[cfg(target_os = "windows")]
    return ("cmd", &["/C", "start", ""]);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return ("xdg-open", &[]);
}

pub fn console(args: &ConsoleArgs, cli: &Cli) -> CmdResult {
    let ctx = crate::auth::auth_context(cli)?;
    let url = resolve_url(args, &ctx.api_url);

    if args.print {
        println!("{url}");
        return Ok(0);
    }

    // Always name the URL, so a headless box (or a machine with no registered
    // handler) still gets something usable — the same courtesy the device flow
    // extends when it prints the verification URI.
    eprintln!("opening {url}");

    let (program, prefix) = opener();
    match Command::new(program).args(prefix).arg(&url).status() {
        Ok(status) if status.success() => Ok(0),
        // A missing or failing opener is not worth an error exit: the user has
        // the URL on stderr and can click or paste it.
        Ok(status) => {
            eprintln!("warning: {program} exited with {status}; open the URL above manually");
            Ok(0)
        }
        Err(e) => {
            eprintln!("warning: could not run {program} ({e}); open the URL above manually");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(url: Option<&str>, path: Option<&str>) -> ConsoleArgs {
        ConsoleArgs {
            path: path.map(str::to_string),
            url: url.map(str::to_string),
            print: false,
        }
    }

    /// An unreachable api_url stands in for "discovery failed", which must fall
    /// back to the compiled-in default rather than erroring.
    const UNREACHABLE: &str = "http://127.0.0.1:1";

    #[test]
    fn falls_back_to_the_default_when_discovery_is_unreachable() {
        assert_eq!(
            resolve_url(&args(None, None), UNREACHABLE),
            tm_config::DEFAULT_PORTAL_URL
        );
    }

    #[test]
    fn url_override_wins_and_never_needs_discovery() {
        assert_eq!(
            resolve_url(
                &args(Some("https://portal.example.test"), None),
                UNREACHABLE
            ),
            "https://portal.example.test"
        );
    }

    #[test]
    fn path_is_joined_with_exactly_one_slash() {
        let base = Some("https://portal.example.test/");
        assert_eq!(
            resolve_url(&args(base, Some("/org/acme/modules")), UNREACHABLE),
            "https://portal.example.test/org/acme/modules"
        );
        assert_eq!(
            resolve_url(&args(base, Some("settings")), UNREACHABLE),
            "https://portal.example.test/settings"
        );
        // An empty path must not leave a trailing slash.
        assert_eq!(
            resolve_url(&args(base, Some("")), UNREACHABLE),
            "https://portal.example.test"
        );
    }
}
