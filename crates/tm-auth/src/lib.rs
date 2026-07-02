//! Authentication for the Terramantle CLI (SPEC §5).
//!
//! Responsibilities:
//!   - bootstrap discovery of the OIDC config ([`discovery`]);
//!   - auth-mode detection ([`mode`]);
//!   - the four acquisition flows ([`flows`]): raw token, client credentials,
//!     CI OIDC (GitHub/GitLab), and the RFC 8628 device flow;
//!   - keyring storage of device tokens ([`store`]);
//!   - local JWT decode for `whoami` ([`jwt`]).
//!
//! Tokens are secrets: nothing in this crate logs a token at any verbosity
//! (rubric 7).

pub mod discovery;
pub mod flows;
pub mod jwt;
pub mod mode;
pub mod store;

pub use discovery::Discovery;
pub use jwt::{Claims, TokenType};
pub use mode::AuthMode;

use tm_api::ApiError;

/// Exit code for auth failures (§9).
pub const EXIT_AUTH: i32 = 5;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("could not fetch discovery document: {0}")]
    Discovery(#[source] ApiError),
    #[error("token exchange failed: {0}")]
    TokenExchange(#[source] ApiError),
    #[error("{0}")]
    MissingCiToken(&'static str),
    #[error("not authenticated; run `terramantle auth login` or set TERRAMANTLE_TOKEN")]
    NotAuthenticated,
    #[error("device login not yet available; use a bot token or CI OIDC")]
    DeviceUnavailable,
    #[error("device authorization expired before it was approved")]
    DeviceExpired,
    #[error("malformed JWT: {0}")]
    MalformedJwt(&'static str),
    #[error("keyring error: {0}")]
    Keyring(String),
}

impl AuthError {
    /// The process exit code this error maps to (§9). All auth failures are 5.
    pub fn exit_code(&self) -> i32 {
        EXIT_AUTH
    }
}

/// Everything the auth layer needs from the resolved config, decoupled from
/// `tm_config` so tests can construct it freely.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Resolved API base URL.
    pub api_url: String,
    /// `TERRAMANTLE_OIDC_ISSUER` override, if any.
    pub issuer_override: Option<String>,
    /// `TERRAMANTLE_AUDIENCE` override, if any.
    pub audience_override: Option<String>,
    /// Resolved auth mode (post detection).
    pub mode: AuthMode,
}

/// Read the process env vars the flows need. Split out so `resolve_token` stays
/// a thin coordinator.
fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Resolve a bearer token for authed commands, following the §5 order:
/// raw env token → client credentials → CI OIDC (github/gitlab) →
/// stored keyring token (device) → `NotAuthenticated`.
///
/// `ctx.mode` already reflects detection/override; this dispatches on it and
/// falls through to the keyring for the interactive/device case.
pub fn resolve_token(ctx: &AuthContext) -> Result<String, AuthError> {
    match ctx.mode {
        AuthMode::Raw => env("TERRAMANTLE_TOKEN")
            .filter(|s| !s.is_empty())
            .ok_or(AuthError::NotAuthenticated),
        AuthMode::ClientCredentials => {
            let disco = discovery::fetch(&ctx.api_url)?;
            let issuer = disco.issuer(ctx.issuer_override.as_deref()).to_string();
            let audience = disco.audience(ctx.audience_override.as_deref()).to_string();
            let client_id = env("TERRAMANTLE_CLIENT_ID").ok_or(AuthError::NotAuthenticated)?;
            let client_secret =
                env("TERRAMANTLE_CLIENT_SECRET").ok_or(AuthError::NotAuthenticated)?;
            flows::client_credentials(disco, &issuer, &audience, &client_id, &client_secret)
        }
        AuthMode::GitHub => {
            let disco = discovery::fetch(&ctx.api_url)?;
            let audience = disco.audience(ctx.audience_override.as_deref()).to_string();
            flows::github_oidc(env, &audience)
        }
        AuthMode::GitLab => flows::gitlab_oidc(env),
        AuthMode::Device => match store::load(&ctx.api_url)? {
            Some(t) => Ok(t.access_token),
            None => Err(AuthError::NotAuthenticated),
        },
    }
}

/// `auth login`: run the device flow and persist the token to the keyring.
/// Gated on `device_client_id != null`. In CI, callers should not invoke this
/// (they acquire ambient tokens instead) — see the command wiring.
pub fn login(ctx: &AuthContext) -> Result<(), AuthError> {
    let disco = discovery::fetch(&ctx.api_url)?;
    let issuer = disco.issuer(ctx.issuer_override.as_deref()).to_string();
    let device_client_id = disco
        .oidc
        .device_client_id
        .clone()
        .ok_or(AuthError::DeviceUnavailable)?;
    let token = flows::device_flow(&issuer, &device_client_id)?;
    store::save(&ctx.api_url, &token)?;
    Ok(())
}

/// `auth logout`: clear the stored device token.
pub fn logout(api_url: &str) -> Result<(), AuthError> {
    store::clear(api_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_auth_errors_map_to_exit_5() {
        assert_eq!(AuthError::NotAuthenticated.exit_code(), 5);
        assert_eq!(AuthError::DeviceUnavailable.exit_code(), 5);
        assert_eq!(AuthError::DeviceExpired.exit_code(), 5);
    }
}
