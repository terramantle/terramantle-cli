//! Minimal shared HTTP client (SPEC §2 stack: `ureq`, blocking, no tokio).
//!
//! This is the foundation the auth slice (§10 slice 2) needs: a base URL, an
//! optional bearer token, and JSON GET/POST helpers. The full typed endpoint
//! surface (one fn per endpoint, §7) is fleshed out in slice 3 — this only lays
//! the groundwork.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A thin wrapper over a `ureq::Agent` carrying a base URL and an optional
/// bearer token. Tokens are never logged (§7 rubric 7).
#[derive(Clone)]
pub struct HttpClient {
    agent: ureq::Agent,
    base_url: String,
    bearer: Option<String>,
}

/// Errors from the HTTP layer. The full `error`-string → exit-code map (§9)
/// lands in slice 3; here we keep enough structure for auth to distinguish a
/// transport failure from an HTTP status.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A non-2xx HTTP response. `body` carries the raw response text (may be a
    /// JSON `{error, message}` envelope) for the caller to map.
    #[error("HTTP {status} from {url}: {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
    },
    /// Transport-level failure (DNS, TLS, connection, timeout).
    #[error("request to {url} failed: {source}")]
    Transport {
        url: String,
        #[source]
        source: Box<ureq::Transport>,
    },
    /// Response body could not be read or deserialized.
    #[error("failed to read response from {url}: {source}")]
    Body {
        url: String,
        #[source]
        source: std::io::Error,
    },
    /// JSON (de)serialization failure.
    #[error("failed to parse JSON from {url}: {source}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

impl ApiError {
    /// The HTTP status, if this is a `Status` error.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Status { status, .. } => Some(*status),
            _ => None,
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
            bearer: None,
        }
    }

    /// Attach a bearer token used on every request.
    #[must_use]
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
    }

    /// Join `path` onto the base URL. Absolute (`http…`) paths pass through so
    /// callers can hit discovered endpoints on a different host (e.g. the OIDC
    /// issuer).
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
        if let Some(token) = &self.bearer {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        req
    }

    /// GET `path` and deserialize the JSON body.
    pub fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.url(path);
        let req = self.apply_auth(self.agent.get(&url));
        Self::send_json(req, &url)
    }

    /// POST a JSON body to `path` and deserialize the JSON response.
    pub fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path);
        let req = self.apply_auth(self.agent.post(&url));
        let value = serde_json::to_value(body).map_err(|source| ApiError::Json {
            url: url.clone(),
            source,
        })?;
        Self::response_json(req.send_json(value), &url)
    }

    fn send_json<T: DeserializeOwned>(req: ureq::Request, url: &str) -> Result<T, ApiError> {
        Self::response_json(req.call(), url)
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
                Err(ApiError::Status {
                    status,
                    url: url.to_string(),
                    body,
                })
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
