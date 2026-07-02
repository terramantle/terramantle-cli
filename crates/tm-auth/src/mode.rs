//! Auth-mode detection (SPEC §5). Detection is a pure function of an environment
//! lookup plus an optional explicit override, so it is unit-tested with an
//! injected map and never touches the process environment directly.
//!
//! Precedence when `auto` (highest first):
//!   1. `TERRAMANTLE_TOKEN` set                       → raw token
//!   2. `TERRAMANTLE_CLIENT_ID` + `_CLIENT_SECRET`    → client credentials
//!   3. `GITHUB_ACTIONS=true`                         → github OIDC
//!   4. `GITLAB_CI=true`                              → gitlab OIDC
//!   5. else (interactive)                            → device flow

/// A resolved authentication mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// `TERRAMANTLE_TOKEN` used verbatim as bearer.
    Raw,
    /// Client-credentials exchange (`TERRAMANTLE_CLIENT_ID`/`_SECRET`).
    ClientCredentials,
    /// GitHub Actions ambient OIDC token.
    GitHub,
    /// GitLab CI ID token.
    GitLab,
    /// Interactive device flow (keyring-backed).
    Device,
}

impl AuthMode {
    /// Parse an explicit override value (`--auth-mode` / `TERRAMANTLE_AUTH_MODE`).
    /// `auto` returns `None` (defer to detection).
    pub fn parse_override(s: &str) -> Result<Option<AuthMode>, String> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(None),
            "token" => Ok(Some(AuthMode::Raw)),
            "client" => Ok(Some(AuthMode::ClientCredentials)),
            "github" => Ok(Some(AuthMode::GitHub)),
            "gitlab" => Ok(Some(AuthMode::GitLab)),
            "device" => Ok(Some(AuthMode::Device)),
            other => Err(format!(
                "invalid auth mode '{other}' (expected auto|token|client|github|gitlab|device)"
            )),
        }
    }
}

fn is_set(get: &impl Fn(&str) -> Option<String>, key: &str) -> bool {
    get(key).map(|v| !v.is_empty()).unwrap_or(false)
}

fn is_true(get: &impl Fn(&str) -> Option<String>, key: &str) -> bool {
    get(key).as_deref() == Some("true")
}

/// Detect the auth mode from an environment lookup, honouring an explicit
/// override. An override short-circuits detection entirely.
pub fn detect(get: impl Fn(&str) -> Option<String>, override_mode: Option<AuthMode>) -> AuthMode {
    if let Some(m) = override_mode {
        return m;
    }
    if is_set(&get, "TERRAMANTLE_TOKEN") {
        return AuthMode::Raw;
    }
    if is_set(&get, "TERRAMANTLE_CLIENT_ID") && is_set(&get, "TERRAMANTLE_CLIENT_SECRET") {
        return AuthMode::ClientCredentials;
    }
    if is_true(&get, "GITHUB_ACTIONS") {
        return AuthMode::GitHub;
    }
    if is_true(&get, "GITLAB_CI") {
        return AuthMode::GitLab;
    }
    AuthMode::Device
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn raw_token_wins_over_everything() {
        let get = env(&[
            ("TERRAMANTLE_TOKEN", "abc"),
            ("TERRAMANTLE_CLIENT_ID", "id"),
            ("TERRAMANTLE_CLIENT_SECRET", "sec"),
            ("GITHUB_ACTIONS", "true"),
            ("GITLAB_CI", "true"),
        ]);
        assert_eq!(detect(get, None), AuthMode::Raw);
    }

    #[test]
    fn client_credentials_over_ci() {
        let get = env(&[
            ("TERRAMANTLE_CLIENT_ID", "id"),
            ("TERRAMANTLE_CLIENT_SECRET", "sec"),
            ("GITHUB_ACTIONS", "true"),
        ]);
        assert_eq!(detect(get, None), AuthMode::ClientCredentials);
    }

    #[test]
    fn client_credentials_needs_both_halves() {
        let get = env(&[("TERRAMANTLE_CLIENT_ID", "id"), ("GITHUB_ACTIONS", "true")]);
        // Only client_id → falls through to github.
        assert_eq!(detect(get, None), AuthMode::GitHub);
    }

    #[test]
    fn github_over_gitlab() {
        let get = env(&[("GITHUB_ACTIONS", "true"), ("GITLAB_CI", "true")]);
        assert_eq!(detect(get, None), AuthMode::GitHub);
    }

    #[test]
    fn gitlab_detected() {
        let get = env(&[("GITLAB_CI", "true")]);
        assert_eq!(detect(get, None), AuthMode::GitLab);
    }

    #[test]
    fn empty_env_is_device() {
        let get = env(&[]);
        assert_eq!(detect(get, None), AuthMode::Device);
    }

    #[test]
    fn empty_string_env_is_not_set() {
        let get = env(&[("TERRAMANTLE_TOKEN", "")]);
        assert_eq!(detect(get, None), AuthMode::Device);
    }

    #[test]
    fn ci_flags_must_be_literally_true() {
        let get = env(&[("GITHUB_ACTIONS", "1")]);
        assert_eq!(detect(get, None), AuthMode::Device);
    }

    #[test]
    fn override_short_circuits_detection() {
        let get = env(&[("TERRAMANTLE_TOKEN", "abc")]);
        assert_eq!(detect(get, Some(AuthMode::Device)), AuthMode::Device);
    }

    #[test]
    fn parse_override_maps_all_variants() {
        assert_eq!(AuthMode::parse_override("auto").unwrap(), None);
        assert_eq!(
            AuthMode::parse_override("token").unwrap(),
            Some(AuthMode::Raw)
        );
        assert_eq!(
            AuthMode::parse_override("client").unwrap(),
            Some(AuthMode::ClientCredentials)
        );
        assert_eq!(
            AuthMode::parse_override("GitHub").unwrap(),
            Some(AuthMode::GitHub)
        );
        assert_eq!(
            AuthMode::parse_override("gitlab").unwrap(),
            Some(AuthMode::GitLab)
        );
        assert_eq!(
            AuthMode::parse_override("device").unwrap(),
            Some(AuthMode::Device)
        );
        assert!(AuthMode::parse_override("nope").is_err());
    }
}
