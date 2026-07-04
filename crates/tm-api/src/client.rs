//! Shared HTTP client (SPEC §2 stack: `ureq`, blocking, no tokio).
//!
//! A base URL, an optional bearer token, and JSON/raw request helpers. The typed
//! endpoint surface (§7) lives in the `providers`/`modules`/`state`/`orgs`
//! modules, which call these helpers. Tokens are never logged (rubric 7).
//!
//! ## Refresh-on-401 (§5, deferred from slice 2)
//! A device (keyring) token can be refreshed. The client optionally holds a
//! [`RefreshHook`]: on a 401 it invokes the hook once, and — if the hook yields a
//! fresh bearer — swaps it in and retries the request a single time. Non-device
//! tokens supply no hook, so a 401 simply propagates (→ exit 5). The refresh
//! mechanics (OIDC exchange + keyring persistence) live in `tm-auth`, injected
//! here so `tm-api` stays free of a keyring dependency.

use std::cell::RefCell;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::ApiError;

/// Invoked once on a 401. Returns a freshly rotated bearer to retry with, or
/// `None` to give up (the 401 then propagates). Implementations persist the new
/// token themselves (e.g. to the keyring).
pub type RefreshHook = Box<dyn Fn() -> Option<String>>;

/// A thin wrapper over a `ureq::Agent` carrying a base URL and an optional
/// bearer token.
pub struct HttpClient {
    agent: ureq::Agent,
    base_url: String,
    bearer: RefCell<Option<String>>,
    refresh: Option<RefreshHook>,
}

impl Clone for HttpClient {
    /// Clones without the refresh hook (hooks are not `Clone`). Cloned clients
    /// therefore do not auto-refresh — acceptable, as refresh is only wired on
    /// the primary command client.
    fn clone(&self) -> Self {
        Self {
            agent: self.agent.clone(),
            base_url: self.base_url.clone(),
            bearer: RefCell::new(self.bearer.borrow().clone()),
            refresh: None,
        }
    }
}

impl HttpClient {
    /// Build a client for `base_url` (no trailing slash required).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .user_agent(concat!("terramantle/", env!("CARGO_PKG_VERSION")))
                .build(),
            base_url: base_url.into(),
            bearer: RefCell::new(None),
            refresh: None,
        }
    }

    /// Attach a bearer token used on every request.
    #[must_use]
    pub fn with_bearer(self, token: impl Into<String>) -> Self {
        *self.bearer.borrow_mut() = Some(token.into());
        self
    }

    /// Attach a refresh hook (see module docs). Only the device flow supplies one.
    #[must_use]
    pub fn with_refresh(mut self, hook: RefreshHook) -> Self {
        self.refresh = Some(hook);
        self
    }

    /// Join `path` onto the base URL. Absolute (`http…`) paths pass through so
    /// callers can hit a different host (e.g. the OIDC issuer).
    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            return path.to_string();
        }
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn apply_auth(&self, mut req: ureq::Request) -> ureq::Request {
        if let Some(token) = self.bearer.borrow().as_ref() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        req
    }

    // ── JSON GET ───────────────────────────────────────────────────────────────

    /// GET `path` and deserialize the JSON body.
    pub fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.url(path);
        self.with_retry(&url, |c| {
            let req = c.apply_auth(c.agent.get(&url));
            Self::response_json(req.call(), &url)
        })
    }

    /// GET `path` with query params and deserialize the JSON body.
    pub fn get_json_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        self.with_retry(&url, |c| {
            let mut req = c.apply_auth(c.agent.get(&url));
            for (k, v) in query {
                req = req.query(k, v);
            }
            Self::response_json(req.call(), &url)
        })
    }

    // ── JSON POST ──────────────────────────────────────────────────────────────

    /// POST a JSON body to `path` and deserialize the JSON response.
    pub fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.post_json_headers(path, body, &[])
    }

    /// POST a JSON body with extra headers (e.g. `Idempotency-Key`).
    pub fn post_json_headers<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        headers: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let value = serde_json::to_value(body).map_err(|source| ApiError::Json {
            url: url.clone(),
            source,
        })?;
        self.with_retry(&url, |c| {
            let mut req = c.apply_auth(c.agent.post(&url));
            for (k, v) in headers {
                req = req.set(k, v);
            }
            Self::response_json(req.send_json(value.clone()), &url)
        })
    }

    // ── form POST ──────────────────────────────────────────────────────────────

    /// POST an `application/x-www-form-urlencoded` body to `path` and deserialize
    /// the JSON response. OAuth 2.0 token/device endpoints (RFC 6749/8628) require
    /// form encoding, not JSON - `post_json` would set `application/json`, and the
    /// issuer then parses no form fields (e.g. Zitadel: "client_id must be provided").
    pub fn post_form<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        self.with_retry(&url, |c| {
            let req = c.apply_auth(c.agent.post(&url));
            Self::response_json(req.send_form(params), &url)
        })
    }

    // ── JSON DELETE (with a body) ──────────────────────────────────────────────

    /// DELETE `path` sending a JSON body (§7 N4: force-unlock echoes the lock id).
    pub fn delete_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let value = serde_json::to_value(body).map_err(|source| ApiError::Json {
            url: url.clone(),
            source,
        })?;
        self.with_retry(&url, |c| {
            let req = c.apply_auth(c.agent.delete(&url));
            Self::response_json(req.send_json(value.clone()), &url)
        })
    }

    // ── raw-body PUT (lock-file upload) ────────────────────────────────────────

    /// PUT a raw byte body to `path` with optional extra headers, deserializing
    /// the JSON response (§7 lock push: `--data-binary` of `.terraform.lock.hcl`).
    pub fn put_bytes<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        self.with_retry(&url, |c| {
            let mut req = c
                .apply_auth(c.agent.put(&url))
                .set("Content-Type", "application/octet-stream");
            for (k, v) in headers {
                req = req.set(k, v);
            }
            Self::response_json(req.send_bytes(body), &url)
        })
    }

    // ── retry / refresh plumbing ───────────────────────────────────────────────

    /// Run `send`; on a 401, invoke the refresh hook once, swap the bearer, and
    /// retry `send` a single time. Any other outcome (or no hook) propagates.
    fn with_retry<T>(
        &self,
        _url: &str,
        send: impl Fn(&Self) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let first = send(self);
        // Only a 401 with a refresh hook that yields a fresh token warrants a
        // retry; every other outcome (including a 401 with no/failed refresh)
        // propagates unchanged so the caller sees the original error.
        if let Err(ApiError::Status { status: 401, .. }) = &first {
            if let Some(hook) = &self.refresh {
                if let Some(fresh) = hook() {
                    *self.bearer.borrow_mut() = Some(fresh);
                    return send(self);
                }
            }
        }
        first
    }

    /// Normalize a `ureq` result into our error space and parse the JSON body.
    fn response_json<T: DeserializeOwned>(
        result: Result<ureq::Response, ureq::Error>,
        url: &str,
    ) -> Result<T, ApiError> {
        match result {
            Ok(resp) => Self::read_json(resp, url),
            Err(ureq::Error::Status(status, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                Err(ApiError::from_status(status, url.to_string(), body))
            }
            Err(ureq::Error::Transport(t)) => Err(ApiError::Transport {
                url: url.to_string(),
                source: Box::new(t),
            }),
        }
    }

    fn read_json<T: DeserializeOwned>(resp: ureq::Response, url: &str) -> Result<T, ApiError> {
        let text = resp.into_string().map_err(|source| ApiError::Body {
            url: url.to_string(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| ApiError::Json {
            url: url.to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A 401 the retry path recognizes.
    fn unauthorized() -> ApiError {
        ApiError::from_status(
            401,
            "https://x/api".into(),
            r#"{"error":"invalid_token"}"#.into(),
        )
    }

    #[test]
    fn with_retry_refreshes_once_then_retries_on_401() {
        let refreshed = Cell::new(false);
        let client = HttpClient::new("https://x")
            .with_bearer("stale")
            .with_refresh(Box::new(|| Some("fresh".to_string())));

        // First send yields 401; after refresh swaps the bearer, the second send
        // observes the fresh token and succeeds.
        let out: Result<u8, ApiError> = client.with_retry("https://x/api", |c| {
            if c.bearer.borrow().as_deref() == Some("fresh") {
                refreshed.set(true);
                Ok(7)
            } else {
                Err(unauthorized())
            }
        });
        assert_eq!(out.unwrap(), 7);
        assert!(refreshed.get(), "retry ran with the refreshed bearer");
        assert_eq!(client.bearer.borrow().as_deref(), Some("fresh"));
    }

    #[test]
    fn with_retry_propagates_401_without_a_hook() {
        // No refresh hook (raw/CI token) → the 401 propagates, exit 5.
        let client = HttpClient::new("https://x").with_bearer("raw");
        let calls = Cell::new(0u32);
        let out: Result<u8, ApiError> = client.with_retry("https://x/api", |_c| {
            calls.set(calls.get() + 1);
            Err(unauthorized())
        });
        assert_eq!(out.unwrap_err().exit_code(), 5);
        assert_eq!(calls.get(), 1, "no retry attempted");
    }

    #[test]
    fn with_retry_propagates_when_hook_declines() {
        // Hook returns None (e.g. no stored refresh token) → original 401 stands,
        // and we do NOT re-send.
        let client = HttpClient::new("https://x")
            .with_bearer("stale")
            .with_refresh(Box::new(|| None));
        let calls = Cell::new(0u32);
        let out: Result<u8, ApiError> = client.with_retry("https://x/api", |_c| {
            calls.set(calls.get() + 1);
            Err(unauthorized())
        });
        assert_eq!(out.unwrap_err().exit_code(), 5);
        assert_eq!(calls.get(), 1, "hook declined — no retry send");
    }

    #[test]
    fn with_retry_does_not_touch_non_401_errors() {
        let client = HttpClient::new("https://x")
            .with_bearer("t")
            .with_refresh(Box::new(|| panic!("must not refresh on non-401")));
        let out: Result<u8, ApiError> = client.with_retry("https://x/api", |_c| {
            Err(ApiError::from_status(
                404,
                "https://x/api".into(),
                r#"{"error":"not_found"}"#.into(),
            ))
        });
        assert_eq!(out.unwrap_err().exit_code(), 6);
    }
}
