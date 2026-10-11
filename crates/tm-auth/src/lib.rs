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
    #[error("session expired and could not be refreshed; run `terramantle auth login`")]
    SessionExpired,
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
///
/// One wrinkle in the raw case: `auth env` *exports* `TERRAMANTLE_TOKEN`, so a
/// second `eval "$(terramantle auth env)"` in the same shell is detected as
/// [`AuthMode::Raw`] and lands here carrying the previous run's bearer. A
/// **stale** raw token therefore falls through to the stored credential and is
/// refreshed, so the command's own output can't disable its own refresh. A
/// fresh raw token still wins outright, and the fallback is inert for CI/bots,
/// which have no keyring entry.
pub fn resolve_token(ctx: &AuthContext) -> Result<String, AuthError> {
    match ctx.mode {
        AuthMode::Raw => {
            let raw = env("TERRAMANTLE_TOKEN")
                .filter(|s| !s.is_empty())
                .ok_or(AuthError::NotAuthenticated)?;
            match freshness(&raw) {
                Freshness::Fresh => Ok(raw),
                stale => match store::load(&ctx.api_url) {
                    Ok(Some(stored)) => refreshed_or(ctx, &stored, raw, stale),
                    // No stored credential to refresh with (the CI/bot shape).
                    _ => expired_guard(raw, stale),
                },
            }
        }
        AuthMode::ClientCredentials => {
            let disco = discovery::fetch(&ctx.api_url)?;
            let scope = disco.scopes().to_string();
            let endpoints = oidc_endpoints(ctx, disco)?;
            let client_id = env("TERRAMANTLE_CLIENT_ID").ok_or(AuthError::NotAuthenticated)?;
            let client_secret =
                env("TERRAMANTLE_CLIENT_SECRET").ok_or(AuthError::NotAuthenticated)?;
            flows::client_credentials(
                &endpoints.token_endpoint,
                &scope,
                &client_id,
                &client_secret,
            )
        }
        AuthMode::GitHub => {
            let disco = discovery::fetch(&ctx.api_url)?;
            // CI tokens are minted for the VCS audience (falls back to the main
            // audience when the deployment doesn't distinguish them).
            let audience = disco
                .vcs_audience(ctx.audience_override.as_deref())
                .to_string();
            flows::github_oidc(env, &audience)
        }
        AuthMode::GitLab => flows::gitlab_oidc(env),
        AuthMode::Device => {
            let stored = store::load(&ctx.api_url)?.ok_or(AuthError::NotAuthenticated)?;
            // Proactively rotate when the cached access token is at/near expiry so
            // the bearer handed to `auth token`/`auth env` (and thus Terraform)
            // outlives the handoff.
            match freshness(&stored.access_token) {
                Freshness::Fresh => Ok(stored.access_token),
                stale => {
                    let current = stored.access_token.clone();
                    refreshed_or(ctx, &stored, current, stale)
                }
            }
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

/// How a locally decoded access token stands relative to its own `exp`.
///
/// The distinction that matters is `Skew` vs `Expired`: a refresh failure on a
/// token that is merely *near* expiry is non-fatal (the bearer still works),
/// whereas one on a token already *past* expiry is terminal — emitting a bearer
/// we know is dead only buys an opaque 401 from the server later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freshness {
    /// Comfortably valid — or carrying no locally readable `exp`, in which case
    /// we don't block on local parsing and the server remains the arbiter.
    Fresh,
    /// Still valid, but within [`REFRESH_SKEW_SECS`] of `exp`.
    Skew,
    /// Past `exp`.
    Expired,
}

/// Classify a JWT by its `exp`. A token with no `exp`, or one we can't decode
/// locally, is [`Freshness::Fresh`] — see the enum's note.
fn freshness(access_token: &str) -> Freshness {
    let exp = match jwt::decode_claims(access_token) {
        Ok(claims) => match claims.exp {
            Some(exp) => exp,
            None => return Freshness::Fresh,
        },
        Err(_) => return Freshness::Fresh,
    };
    let now = now_unix();
    if now >= exp {
        Freshness::Expired
    } else if now + REFRESH_SKEW_SECS >= exp {
        Freshness::Skew
    } else {
        Freshness::Fresh
    }
}

/// Refresh `stored` and yield the rotated access token, falling back to
/// `current` when the exchange fails. The failure is always narrated (it used to
/// be swallowed, which made a revoked refresh token indistinguishable from a
/// working one) and the fallback is gated on [`expired_guard`].
fn refreshed_or(
    ctx: &AuthContext,
    stored: &store::StoredToken,
    current: String,
    stale: Freshness,
) -> Result<String, AuthError> {
    match try_refresh(ctx, stored) {
        Ok(rotated) => Ok(rotated.access_token),
        Err(e) => {
            warn_refresh_failed(&e);
            expired_guard(current, stale)
        }
    }
}

/// Hand back `current` only if it is still usable. A token past its `exp` with
/// no viable refresh is a dead end, so fail here with an actionable message
/// rather than letting the caller hand it to Terraform.
fn expired_guard(current: String, stale: Freshness) -> Result<String, AuthError> {
    match stale {
        Freshness::Expired => Err(AuthError::SessionExpired),
        _ => Ok(current),
    }
}

/// Narrate a failed refresh on stderr. Never prints a token (rubric 7) — the
/// error carries the token endpoint's own failure, not our credential.
fn warn_refresh_failed(e: &AuthError) {
    eprintln!("warning: token refresh failed: {e}");
}

/// Resolve the issuer's OIDC endpoints (token, device-authorization) from its
/// RFC 8414 metadata. Uses the Terramantle discovery doc's `discovery_url`, or —
/// when the issuer is overridden via `TERRAMANTLE_OIDC_ISSUER` — derives
/// `{issuer}/.well-known/openid-configuration`. This is what keeps the flows free
/// of hardcoded, provider-specific paths.
fn oidc_endpoints(
    ctx: &AuthContext,
    disco: &discovery::Discovery,
) -> Result<&'static discovery::OidcEndpoints, AuthError> {
    match ctx.issuer_override.as_deref() {
        Some(iss) => {
            let url = format!(
                "{}/.well-known/openid-configuration",
                iss.trim_end_matches('/')
            );
            discovery::fetch_oidc_endpoints(&url)
        }
        None => discovery::fetch_oidc_endpoints(&disco.oidc.discovery_url),
    }
}

/// Resolve the device client + token endpoint from discovery, then exchange the
/// stored refresh token once and persist the rotation. Returns the cause on
/// failure (no refresh token, no device client, discovery/exchange failure) so
/// callers can narrate it instead of guessing.
fn try_refresh(
    ctx: &AuthContext,
    stored: &store::StoredToken,
) -> Result<store::StoredToken, AuthError> {
    let disco = discovery::fetch(&ctx.api_url)?;
    let device_client_id = disco
        .oidc
        .device_client_id
        .clone()
        .ok_or(AuthError::DeviceUnavailable)?;
    let scope = disco.scopes().to_string();
    let endpoints = oidc_endpoints(ctx, disco)?;
    refresh_once(
        &ctx.api_url,
        &endpoints.token_endpoint,
        &device_client_id,
        &scope,
        stored,
    )
}

/// Exchange a bundle's refresh token once at `token_endpoint` and persist the
/// rotated bundle. Shared by the proactive path ([`try_refresh`]) and the on-401
/// hook so rotation + persistence live in one place.
fn refresh_once(
    api_url: &str,
    token_endpoint: &str,
    device_client_id: &str,
    scope: &str,
    stored: &store::StoredToken,
) -> Result<store::StoredToken, AuthError> {
    let refresh = stored
        .refresh_token
        .as_deref()
        .ok_or(AuthError::NotAuthenticated)?;
    let rotated = flows::refresh_token(token_endpoint, device_client_id, refresh, scope)?;
    // Persist before returning so the rotation survives the process.
    store::save(api_url, &rotated)?;
    Ok(rotated)
}

/// `auth login`: run the device flow and persist the token to the keyring.
/// Gated on `device_client_id != null`. In CI, callers should not invoke this
/// (they acquire ambient tokens instead) — see the command wiring.
pub fn login(ctx: &AuthContext) -> Result<(), AuthError> {
    let disco = discovery::fetch(&ctx.api_url)?;
    let device_client_id = disco
        .oidc
        .device_client_id
        .clone()
        .ok_or(AuthError::DeviceUnavailable)?;
    let endpoints = oidc_endpoints(ctx, disco)?;
    let device_auth = endpoints
        .device_authorization_endpoint
        .as_deref()
        .ok_or(AuthError::DeviceUnavailable)?;
    let token = flows::device_flow(
        device_auth,
        &endpoints.token_endpoint,
        &device_client_id,
        disco.scopes(),
    )?;
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
    Ok(client_with_token(ctx, token))
}

/// Build a client for an **already-resolved** bearer, fitting the same
/// refresh-on-401 hook [`client`] would. For callers that need both the token
/// and a client (e.g. `whoami`, which decodes the JWT locally) so they don't
/// resolve twice.
///
/// Raw mode gets the hook as well as device mode: a raw bearer may be `auth
/// env`'s own output, backed by the very stored credential that can refresh it.
/// [`build_refresh_hook`] yields `None` when there is no stored credential, so
/// this stays inert for CI and bot tokens.
pub fn client_with_token(ctx: &AuthContext, token: String) -> Client {
    match ctx.mode {
        AuthMode::Device | AuthMode::Raw => match build_refresh_hook(ctx) {
            Some(hook) => Client::with_refresh(ctx.api_url.clone(), token, hook),
            None => Client::new(ctx.api_url.clone(), token),
        },
        _ => Client::new(ctx.api_url.clone(), token),
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
    let device_client_id = disco.oidc.device_client_id.clone()?;
    let scope = disco.scopes().to_string();
    let token_endpoint = oidc_endpoints(ctx, disco).ok()?.token_endpoint.clone();
    let api_url = ctx.api_url.clone();

    Some(Box::new(move || {
        // Re-read on each call so a token rotated by an earlier retry is picked up.
        let stored = store::load(&api_url).ok()??;
        match refresh_once(
            &api_url,
            &token_endpoint,
            &device_client_id,
            &scope,
            &stored,
        ) {
            Ok(rotated) => Some(rotated.access_token),
            Err(e) => {
                warn_refresh_failed(&e);
                None
            }
        }
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
        assert_eq!(AuthError::SessionExpired.exit_code(), 5);
    }

    /// Build an unsigned JWT carrying just `exp` (seconds since epoch).
    fn jwt_with_exp(exp: i64) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let body = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
        format!("{header}.{body}.")
    }

    /// Build an unsigned JWT with no `exp` at all.
    fn jwt_without_exp() -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let b = URL_SAFE_NO_PAD.encode(br#"{"sub":"u"}"#);
        format!("{h}.{b}.")
    }

    #[test]
    fn freshness_separates_expired_from_skew() {
        // Already past exp — terminal if refresh can't recover it.
        assert_eq!(
            freshness(&jwt_with_exp(now_unix() - 10)),
            Freshness::Expired
        );
        // Inside the skew window: still usable, so a failed refresh is survivable.
        assert_eq!(
            freshness(&jwt_with_exp(now_unix() + REFRESH_SKEW_SECS - 5)),
            Freshness::Skew
        );
    }

    #[test]
    fn freshness_fresh_when_valid_or_unparseable_or_no_exp() {
        // Comfortably in the future.
        assert_eq!(
            freshness(&jwt_with_exp(now_unix() + 3600)),
            Freshness::Fresh
        );
        // Undecodable → fresh (server is the arbiter, we don't block locally).
        assert_eq!(freshness("not-a-jwt"), Freshness::Fresh);
        // No exp claim → nothing to reason about.
        assert_eq!(freshness(&jwt_without_exp()), Freshness::Fresh);
    }

    /// The guard is what stops a known-dead bearer reaching Terraform.
    #[test]
    fn expired_guard_fails_only_past_exp() {
        assert!(matches!(
            expired_guard("t".into(), Freshness::Expired),
            Err(AuthError::SessionExpired)
        ));
        // Near-expiry but still valid: keep using it rather than hard-failing.
        assert_eq!(
            expired_guard("t".into(), Freshness::Skew).unwrap(),
            "t".to_string()
        );
        assert_eq!(
            expired_guard("t".into(), Freshness::Fresh).unwrap(),
            "t".to_string()
        );
    }

    /// Both raw-mode cases live in one test on purpose: `resolve_token` reads
    /// the *process* environment, and cargo runs tests in a shared process, so
    /// splitting them would let two tests race over `TERRAMANTLE_TOKEN`.
    ///
    /// A fresh `TERRAMANTLE_TOKEN` must still win outright — the keyring
    /// fall-through added for stale tokens leaves §5 precedence unchanged for
    /// every case that already worked.
    #[test]
    fn raw_mode_precedence() {
        // Unreachable host: proves neither case does network or keyring work.
        let ctx = AuthContext {
            api_url: "http://127.0.0.1:1".to_string(),
            issuer_override: None,
            audience_override: None,
            mode: AuthMode::Raw,
        };

        let token = jwt_with_exp(now_unix() + 3600);
        // SAFETY: the only test in this binary that touches this var, and it is
        // read back before anything else can observe it.
        unsafe { std::env::set_var("TERRAMANTLE_TOKEN", &token) };
        let fresh = resolve_token(&ctx);
        unsafe { std::env::remove_var("TERRAMANTLE_TOKEN") };
        assert_eq!(fresh.unwrap(), token);

        assert!(matches!(
            resolve_token(&ctx),
            Err(AuthError::NotAuthenticated)
        ));
    }
}
