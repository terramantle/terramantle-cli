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

use crate::store::StoredToken;
use crate::AuthError;

/// A minimal OAuth token response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Client-credentials exchange (§5, bot flow): `POST {token_endpoint}` with
/// `grant_type=client_credentials`. The token endpoint is the absolute URL from
/// the issuer's OIDC metadata and the scopes come from discovery — no
/// provider-specific path or parameter is assumed. The token's audience is the
/// IdP's business (configured server-side, typically via a scope mapping), not
/// a request parameter.
pub fn client_credentials(
    token_endpoint: &str,
    scope: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, AuthError> {
    let client = HttpClient::new("");
    let resp: TokenResponse = client
        .post_form(
            token_endpoint,
            &[
                ("grant_type", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("scope", scope),
            ],
        )
        .map_err(AuthError::TokenExchange)?;
    Ok(resp.access_token)
}

/// Refresh a device token (§5, deferred from slice 2): `POST {token_endpoint}`
/// with `grant_type=refresh_token`. Returns the rotated bundle; the refresh token
/// may itself rotate, so we prefer the new one and fall back to the old.
pub fn refresh_token(
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<StoredToken, AuthError> {
    let client = HttpClient::new("");
    let resp: TokenResponse = client
        .post_form(
            token_endpoint,
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
/// `device_client_id != null`. `scope` comes from discovery (see
/// [`crate::discovery::Discovery::scopes`]). Prints the verification URI +
/// user code to stderr, then polls until success or expiry. Returns the stored
/// token bundle.
pub fn device_flow(
    device_authorization_endpoint: &str,
    token_endpoint: &str,
    device_client_id: &str,
    scope: &str,
) -> Result<StoredToken, AuthError> {
    let client = HttpClient::new("");
    let auth: DeviceAuth = client
        .post_form(
            device_authorization_endpoint,
            &[("client_id", device_client_id), ("scope", scope)],
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
        sleep(interval);
        // Checked after the sleep so we never poll past the server's expiry.
        if Instant::now() >= deadline {
            return Err(AuthError::DeviceExpired);
        }
        match client.post_form::<TokenResponse>(token_endpoint, &token_params) {
            Ok(resp) => {
                return Ok(StoredToken {
                    access_token: resp.access_token,
                    refresh_token: resp.refresh_token,
                })
            }
            Err(e) => match poll_disposition(&e) {
                PollDisposition::KeepWaiting => {}
                PollDisposition::SlowDown => interval += Duration::from_secs(5),
                PollDisposition::Expired => return Err(AuthError::DeviceExpired),
                PollDisposition::Fatal => return Err(AuthError::TokenExchange(e)),
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PollDisposition {
    KeepWaiting,
    SlowDown,
    Expired,
    Fatal,
}

/// Interpret a polling error per RFC 8628 §3.5: `authorization_pending` and
/// `slow_down` are non-fatal, `expired_token` maps to the dedicated expiry
/// error, anything else (e.g. `access_denied`) aborts.
fn poll_disposition(err: &ApiError) -> PollDisposition {
    if let ApiError::Status { body, .. } = err {
        if let Ok(PollError { error }) = serde_json::from_str::<PollError>(body) {
            return match error.as_str() {
                "authorization_pending" => PollDisposition::KeepWaiting,
                "slow_down" => PollDisposition::SlowDown,
                "expired_token" => PollDisposition::Expired,
                _ => PollDisposition::Fatal,
            };
        }
    }
    PollDisposition::Fatal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll_err(status: u16, body: &str) -> ApiError {
        // Mirror how the HTTP layer surfaces a non-2xx poll response.
        ApiError::Status {
            status,
            url: "https://iss/token".into(),
            body: body.to_string(),
            parsed: None,
        }
    }

    #[test]
    fn pending_and_slow_down_keep_polling() {
        assert_eq!(
            poll_disposition(&poll_err(400, r#"{"error":"authorization_pending"}"#)),
            PollDisposition::KeepWaiting
        );
        assert_eq!(
            poll_disposition(&poll_err(400, r#"{"error":"slow_down"}"#)),
            PollDisposition::SlowDown
        );
    }

    #[test]
    fn expired_token_maps_to_expired() {
        assert_eq!(
            poll_disposition(&poll_err(400, r#"{"error":"expired_token"}"#)),
            PollDisposition::Expired
        );
    }

    #[test]
    fn denial_and_garbage_are_fatal() {
        assert_eq!(
            poll_disposition(&poll_err(400, r#"{"error":"access_denied"}"#)),
            PollDisposition::Fatal
        );
        assert_eq!(
            poll_disposition(&poll_err(502, "<html>bad gateway</html>")),
            PollDisposition::Fatal
        );
    }

    #[test]
    fn default_interval_is_five_seconds() {
        let auth: DeviceAuth = serde_json::from_str(
            r#"{"device_code":"d","user_code":"U-1","verification_uri":"https://iss/device","expires_in":600}"#,
        )
        .unwrap();
        assert_eq!(auth.interval, 5);
        assert_eq!(auth.verification_uri_complete, None);
    }
}
