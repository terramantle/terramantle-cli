//! Token-acquisition flows (SPEC §5): client credentials, GitHub/GitLab ambient
//! OIDC, and the RFC 8628 device flow. Discovery supplies the issuer/audience;
//! these functions only speak the wire protocol.
//!
//! No token is ever logged here (rubric 7).

use std::io::Write;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tm_api::{ApiError, HttpClient};

use crate::discovery::Discovery;
use crate::store::StoredToken;
use crate::AuthError;

const DEVICE_SCOPE: &str = "openid profile email offline_access";

/// A minimal OAuth token response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Client-credentials exchange (§5, bot flow).
/// `POST {issuer}/oauth/v2/token` with `grant_type=client_credentials`.
pub fn client_credentials(
    disco: &Discovery,
    issuer: &str,
    audience: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, AuthError> {
    let _ = disco;
    let client = HttpClient::new(issuer);
    let resp: TokenResponse = client
        .post_form(
            "/oauth/v2/token",
            &[
                ("grant_type", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("audience", audience),
            ],
        )
        .map_err(AuthError::TokenExchange)?;
    Ok(resp.access_token)
}

/// Refresh a device token (§5, deferred from slice 2). `POST {issuer}/oauth/v2/token`
/// with `grant_type=refresh_token`. Returns the rotated bundle; the refresh token
/// may itself rotate, so we prefer the new one and fall back to the old.
pub fn refresh_token(
    issuer: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<StoredToken, AuthError> {
    let client = HttpClient::new(issuer);
    let resp: TokenResponse = client
        .post_form(
            "/oauth/v2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh_token),
            ],
        )
        .map_err(AuthError::TokenExchange)?;
    Ok(StoredToken {
        access_token: resp.access_token,
        refresh_token: resp
            .refresh_token
            .or_else(|| Some(refresh_token.to_string())),
    })
}

/// GitHub Actions ambient OIDC (§5). Reads `ACTIONS_ID_TOKEN_REQUEST_URL` +
/// `_TOKEN`; errors clearly when absent (needs `id-token: write`).
pub fn github_oidc(
    get: impl Fn(&str) -> Option<String>,
    audience: &str,
) -> Result<String, AuthError> {
    let url = get("ACTIONS_ID_TOKEN_REQUEST_URL").filter(|s| !s.is_empty());
    let req_token = get("ACTIONS_ID_TOKEN_REQUEST_TOKEN").filter(|s| !s.is_empty());
    let (url, req_token) = match (url, req_token) {
        (Some(u), Some(t)) => (u, t),
        _ => {
            return Err(AuthError::MissingCiToken(
                "GitHub OIDC token unavailable: ACTIONS_ID_TOKEN_REQUEST_URL/_TOKEN not set. \
                 Grant the job `permissions: id-token: write`.",
            ))
        }
    };
    let sep = if url.contains('?') { '&' } else { '?' };
    let full = format!("{url}{sep}audience={audience}");
    let client = HttpClient::new("").with_bearer(req_token);

    #[derive(Deserialize)]
    struct IdToken {
        value: String,
    }
    let resp: IdToken = client.get_json(&full).map_err(AuthError::TokenExchange)?;
    Ok(resp.value)
}

/// GitLab CI ID token (§5). Read from `TERRAMANTLE_ID_TOKEN`, which the user
/// configures via an `id_tokens` entry with our audience.
pub fn gitlab_oidc(get: impl Fn(&str) -> Option<String>) -> Result<String, AuthError> {
    get("TERRAMANTLE_ID_TOKEN")
        .filter(|s| !s.is_empty())
        .ok_or(AuthError::MissingCiToken(
            "GitLab OIDC token unavailable: TERRAMANTLE_ID_TOKEN not set. Configure an \
             `id_tokens:` entry with aud set to the Terramantle audience.",
        ))
}

/// RFC 8628 device-authorization response.
#[derive(Debug, Deserialize)]
struct DeviceAuth {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Error envelope while polling the token endpoint (RFC 8628 §3.5).
#[derive(Debug, Deserialize)]
struct PollError {
    error: String,
}

/// Run the RFC 8628 device flow (§5). Gated by the caller on
/// `device_client_id != null`. Prints the verification URI + user code to
/// stderr, then polls until success or expiry. Returns the stored token bundle.
pub fn device_flow(issuer: &str, device_client_id: &str) -> Result<StoredToken, AuthError> {
    let client = HttpClient::new(issuer);
    let auth: DeviceAuth = client
        .post_form(
            "/oauth/v2/device_authorization",
            &[("client_id", device_client_id), ("scope", DEVICE_SCOPE)],
        )
        .map_err(AuthError::TokenExchange)?;

    let mut err = std::io::stderr();
    let _ = writeln!(err, "To authenticate, open:");
    if let Some(complete) = &auth.verification_uri_complete {
        let _ = writeln!(err, "  {complete}");
    }
    let _ = writeln!(err, "  {}", auth.verification_uri);
    let _ = writeln!(err, "and enter code: {}", auth.user_code);

    let deadline = Instant::now() + Duration::from_secs(auth.expires_in);
    let mut interval = Duration::from_secs(auth.interval.max(1));
    let token_params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", auth.device_code.as_str()),
        ("client_id", device_client_id),
    ];

    loop {
        if Instant::now() >= deadline {
            return Err(AuthError::DeviceExpired);
        }
        sleep(interval);
        match client.post_form::<TokenResponse>("/oauth/v2/token", &token_params) {
            Ok(resp) => {
                return Ok(StoredToken {
                    access_token: resp.access_token,
                    refresh_token: resp.refresh_token,
                })
            }
            Err(e) => match poll_disposition(&e) {
                PollDisposition::KeepWaiting => {}
                PollDisposition::SlowDown => interval += Duration::from_secs(5),
                PollDisposition::Fatal => return Err(AuthError::TokenExchange(e)),
            },
        }
    }
}

enum PollDisposition {
    KeepWaiting,
    SlowDown,
    Fatal,
}

/// Interpret a polling error per RFC 8628: `authorization_pending` and
/// `slow_down` are non-fatal; anything else aborts.
fn poll_disposition(err: &ApiError) -> PollDisposition {
    if let ApiError::Status { body, .. } = err {
        if let Ok(PollError { error }) = serde_json::from_str::<PollError>(body) {
            return match error.as_str() {
                "authorization_pending" => PollDisposition::KeepWaiting,
                "slow_down" => PollDisposition::SlowDown,
                _ => PollDisposition::Fatal,
            };
        }
    }
    PollDisposition::Fatal
}
