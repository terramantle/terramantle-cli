//! Running the user's `tofu` / `terraform` binary for the one thing the CLI
//! cannot do itself: rendering a binary plan to `show -json`.
//!
//! The render needs the providers' schemas, which only an initialised working
//! directory has, so `render_plan` runs in `chdir` (where `init` ran). Which
//! binary: `TERRAMANTLE_TF_BIN`, else `tofu` if on PATH, else `terraform`. The
//! process spawn is behind a closure so the command logic is testable without
//! one (rubric 3).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment override for the terraform-compatible binary.
pub const TF_BIN_ENV: &str = "TERRAMANTLE_TF_BIN";

/// Candidates, in preference order, when nothing is configured.
const CANDIDATES: &[&str] = &["tofu", "terraform"];

/// Pick the binary: explicit flag, then `TERRAMANTLE_TF_BIN`, then the first of
/// `tofu` / `terraform` found on PATH.
pub fn detect_tf_bin(flag: Option<&str>) -> Option<String> {
    detect_with(flag, std::env::var(TF_BIN_ENV).ok().as_deref(), on_path)
}

/// Pure core of [`detect_tf_bin`]: `on_path` reports whether a candidate exists.
pub fn detect_with(
    flag: Option<&str>,
    env: Option<&str>,
    on_path: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(f) = flag.filter(|s| !s.trim().is_empty()) {
        return Some(f.to_string());
    }
    if let Some(e) = env.filter(|s| !s.trim().is_empty()) {
        return Some(e.to_string());
    }
    CANDIDATES
        .iter()
        .find(|c| on_path(c))
        .map(|c| c.to_string())
}

fn on_path(bin: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let p = dir.join(bin);
        p.is_file() || cfg!(windows) && dir.join(format!("{bin}.exe")).is_file()
    })
}

/// Why a render failed, with the fix in the message.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no terraform-compatible binary found (looked for tofu, terraform); set {TF_BIN_ENV} or pass --tf-bin")]
    NoBinary,
    #[error("could not run {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{bin} show -json {plan} failed (exit {code}):\n{stderr}")]
    Failed {
        bin: String,
        plan: PathBuf,
        code: i32,
        stderr: String,
    },
}

/// `<bin> show -json <plan>` in `chdir`, returning the render text.
pub fn render_plan(bin: &str, chdir: &Path, plan: &Path) -> Result<String, RenderError> {
    let out = Command::new(bin)
        .arg("show")
        .arg("-json")
        .arg(plan)
        .current_dir(chdir)
        .output()
        .map_err(|source| RenderError::Spawn {
            bin: bin.to_string(),
            source,
        })?;
    if !out.status.success() {
        return Err(RenderError::Failed {
            bin: bin.to_string(),
            plan: plan.to_path_buf(),
            code: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_beats_env_beats_path() {
        assert_eq!(
            detect_with(Some("/opt/tf"), Some("tofu"), |_| true).as_deref(),
            Some("/opt/tf")
        );
        assert_eq!(
            detect_with(None, Some("/x/tofu"), |_| false).as_deref(),
            Some("/x/tofu")
        );
        assert_eq!(
            detect_with(None, None, |b| b == "terraform").as_deref(),
            Some("terraform")
        );
        assert_eq!(detect_with(None, None, |_| true).as_deref(), Some("tofu"));
        assert_eq!(detect_with(Some("  "), None, |_| false), None);
    }
}
