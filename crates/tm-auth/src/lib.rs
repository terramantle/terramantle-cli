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

use tm_api::{ApiError, Client, RefreshHook};

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
        AuthMode::Device => {
            let stored = store::load(&ctx.api_url)?.ok_or(AuthError::NotAuthenticated)?;
            // Proactively rotate when the cached access token is at/near expiry so
            // the bearer handed to `auth token`/`auth env` (and thus Terraform)
            // outlives the handoff. A failed refresh is non-fatal: fall back to the
            // current token and let the server be the final arbiter.
            if is_stale(&stored.access_token) {
                if let Some(rotated) = try_refresh(ctx, &stored) {
                    return Ok(rotated.access_token);
                }
            }
            Ok(stored.access_token)
        }
    }
}

/// Clock skew before `exp` at which a device access token is treated as due for
/// proactive refresh, so a token handed off (e.g. to Terraform) doesn't expire
/// mid-use.
const REFRESH_SKEW_SECS: i64 = 60;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether a JWT is expired or within [`REFRESH_SKEW_SECS`] of expiring. A token
/// with no `exp`, or one we can't decode locally, is treated as *not* stale — we
/// don't block on local parsing; the server remains the final arbiter.
fn is_stale(access_token: &str) -> bool {
    match jwt::decode_claims(access_token) {
        Ok(claims) => claims
            .exp
            .is_some_and(|exp| now_unix() + REFRESH_SKEW_SECS >= exp),
        Err(_) => false,
    }
}

/// Resolve the device client + issuer from discovery, then exchange the stored
/// refresh token once and persist the rotation. `None` when refresh isn't
/// possible (no refresh token, no device client, discovery/exchange failure).
fn try_refresh(ctx: &AuthContext, stored: &store::StoredToken) -> Option<store::StoredToken> {
    let disco = discovery::fetch(&ctx.api_url).ok()?;
    let issuer = disco.issuer(ctx.issuer_override.as_deref()).to_string();
    let device_client_id = disco.oidc.device_client_id.clone()?;
    refresh_once(&ctx.api_url, &issuer, &device_client_id, stored)
}

/// Exchange a bundle's refresh token once and persist the rotated bundle. Shared
/// by the proactive path ([`try_refresh`]) and the on-401 hook so rotation +
/// persistence live in one place. `None` when there's no refresh token or the
/// exchange/persist fails.
fn refresh_once(
    api_url: &str,
    issuer: &str,
    device_client_id: &str,
    stored: &store::StoredToken,
) -> Option<store::StoredToken> {
    let refresh = stored.refresh_token.as_deref()?;
    let rotated = flows::refresh_token(issuer, device_client_id, refresh).ok()?;
    // Persist before returning so the rotation survives the process.
    store::save(api_url, &rotated).ok()?;
    Some(rotated)
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

/// Build a `tm_api::Client` for an authed command, resolving the bearer per §5.
///
/// For the **device** flow the client is fitted with a refresh-on-401 hook
/// (§5, deferred from slice 2): a 401 triggers one `grant_type=refresh_token`
/// exchange, the rotated token is persisted to the keyring, and the request is
/// retried once. Every **non-device** flow (raw/CI/client-creds) gets a plain
/// client, so a 401 simply propagates → exit 5. Refresh is contained entirely in
/// the client request path.
pub fn client(ctx: &AuthContext) -> Result<Client, AuthError> {
    let token = resolve_token(ctx)?;
    match ctx.mode {
        AuthMode::Device => match build_refresh_hook(ctx) {
            Some(hook) => Ok(Client::with_refresh(ctx.api_url.clone(), token, hook)),
            None => Ok(Client::new(ctx.api_url.clone(), token)),
        },
        _ => Ok(Client::new(ctx.api_url.clone(), token)),
    }
}

/// Build the device refresh-on-401 hook, or `None` when there is nothing to
/// refresh with (no stored refresh token, or discovery has no device client).
/// The hook loads the stored bundle, exchanges the refresh token once, persists
/// the rotated bundle, and yields the new access token.
fn build_refresh_hook(ctx: &AuthContext) -> Option<RefreshHook> {
    // Both a stored refresh token and a device client id are required; if either
    // is missing there is no viable refresh and the 401 should just propagate.
    let stored = store::load(&ctx.api_url).ok()??;
    stored.refresh_token.as_ref()?;
    let disco = discovery::fetch(&ctx.api_url).ok()?;
    let issuer = disco.issuer(ctx.issuer_override.as_deref()).to_string();
    let device_client_id = disco.oidc.device_client_id.clone()?;
    let api_url = ctx.api_url.clone();

    Some(Box::new(move || {
        // Re-read on each call so a token rotated by an earlier retry is picked up.
        let stored = store::load(&api_url).ok()??;
        let rotated = refresh_once(&api_url, &issuer, &device_client_id, &stored)?;
        Some(rotated.access_token)
    }))
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

    /// Build an unsigned JWT carrying just `exp` (seconds since epoch).
    fn jwt_with_exp(exp: i64) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let body = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
        format!("{header}.{body}.")
    }

    #[test]
    fn is_stale_true_when_expired_or_within_skew() {
        // Already expired.
        assert!(is_stale(&jwt_with_exp(now_unix() - 10)));
        // Inside the skew window (expires in < REFRESH_SKEW_SECS).
        assert!(is_stale(&jwt_with_exp(now_unix() + REFRESH_SKEW_SECS - 5)));
    }

    #[test]
    fn is_stale_false_when_fresh_or_unparseable_or_no_exp() {
        // Comfortably in the future.
        assert!(!is_stale(&jwt_with_exp(now_unix() + 3600)));
        // Undecodable → not stale (server is the arbiter, we don't block locally).
        assert!(!is_stale("not-a-jwt"));
        // No exp claim → nothing to reason about.
        assert!(!is_stale(&{
            use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
            let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
            let b = URL_SAFE_NO_PAD.encode(br#"{"sub":"u"}"#);
            format!("{h}.{b}.")
        }));
    }
}
