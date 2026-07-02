//! API error type + the §9 error-string → exit-code map.
//!
//! The worker returns `{ error: <code>, message, … }` on failure. `ApiError`
//! carries the HTTP status and, when the body parsed, the `error`/`message`
//! fields (plus any lock-holder fields for display). `exit_code()` implements
//! the §9 table **exactly** and is unit-tested per case.

use serde::Deserialize;

/// The `{ error, message, … }` envelope the worker returns on failure. Lock
/// conflicts additionally carry holder fields; we keep the ones useful for a
/// human-facing message and tolerate the rest.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ErrorBody {
    /// Machine-readable error code, e.g. `not_found`, `locked`.
    #[serde(default)]
    pub error: Option<String>,
    /// Human-readable message; always preserved for display.
    #[serde(default)]
    pub message: Option<String>,
    /// Lock holder, present on `locked` conflicts (§9 note).
    #[serde(default)]
    pub who: Option<String>,
    /// Lock id, present on some lock conflicts.
    #[serde(default)]
    pub lock_id: Option<String>,
    /// Operation the holding lock is performing.
    #[serde(default)]
    pub operation: Option<String>,
}

/// Errors from the API layer.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A non-2xx HTTP response. `body` carries the raw text; `parsed` is the
    /// `{error, message}` envelope when it deserialized.
    #[error("HTTP {status} from {url}: {}", .parsed.as_ref().and_then(|p| p.message.clone()).unwrap_or_else(|| body.clone()))]
    Status {
        status: u16,
        url: String,
        body: String,
        /// Boxed to keep the `ApiError` enum small (clippy `result_large_err`).
        parsed: Option<Box<ErrorBody>>,
    },
    /// Transport-level failure (DNS, TLS, connection, timeout).
    #[error("request to {url} failed: {source}")]
    Transport {
        url: String,
        #[source]
        source: Box<ureq::Transport>,
    },
    /// Response body could not be read.
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
    /// Build a `Status` error, parsing the body as an `{error, message}` envelope.
    pub(crate) fn from_status(status: u16, url: String, body: String) -> Self {
        let parsed = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .filter(|b| b.error.is_some() || b.message.is_some())
            .map(Box::new);
        ApiError::Status {
            status,
            url,
            body,
            parsed,
        }
    }

    /// The HTTP status, if this is a `Status` error.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The machine-readable `error` code from the parsed body, if any.
    pub fn error_code(&self) -> Option<&str> {
        match self {
            ApiError::Status {
                parsed: Some(b), ..
            } => b.error.as_deref(),
            _ => None,
        }
    }

    /// The human-readable `message` from the parsed body, if any. Preserved for
    /// display (§9: "preserve `message`").
    pub fn message(&self) -> Option<&str> {
        match self {
            ApiError::Status {
                parsed: Some(b), ..
            } => b.message.as_deref(),
            _ => None,
        }
    }

    /// The lock holder from the parsed body, when the error is a lock conflict.
    pub fn lock_holder(&self) -> Option<&str> {
        match self {
            ApiError::Status {
                parsed: Some(b), ..
            } => b.who.as_deref(),
            _ => None,
        }
    }

    /// Map this error to a process exit code per the §9 table. The `error` string
    /// is authoritative; when the body did not parse we fall back to `1`.
    ///
    /// | `error` | exit |
    /// |---|---|
    /// | `unauthorized`, `invalid_token`, `forbidden` | 5 |
    /// | `not_found`, `feature_disabled` (a 404) | 6 |
    /// | `bad_request` | 2 |
    /// | `locked`, `serial_conflict` (409) | 7 |
    /// | `payload_too_large` | 1 |
    /// | unmapped / non-JSON | 1 |
    pub fn exit_code(&self) -> i32 {
        match self.error_code() {
            Some("unauthorized" | "invalid_token" | "forbidden") => 5,
            Some("not_found" | "feature_disabled") => 6,
            Some("bad_request") => 2,
            Some("locked" | "serial_conflict") => 7,
            Some("payload_too_large") => 1,
            // Unmapped `error` string or non-JSON/absent body → generic (§9).
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `Status` error from a crafted JSON body, as if the worker returned it.
    fn status(http: u16, body: &str) -> ApiError {
        ApiError::from_status(http, "https://example/api".into(), body.to_string())
    }

    #[test]
    fn unauthorized_maps_to_5() {
        assert_eq!(
            status(401, r#"{"error":"unauthorized","message":"no token"}"#).exit_code(),
            5
        );
    }

    #[test]
    fn invalid_token_maps_to_5() {
        assert_eq!(
            status(401, r#"{"error":"invalid_token","message":"expired"}"#).exit_code(),
            5
        );
    }

    #[test]
    fn forbidden_maps_to_5() {
        assert_eq!(
            status(403, r#"{"error":"forbidden","message":"need admin"}"#).exit_code(),
            5
        );
    }

    #[test]
    fn not_found_maps_to_6() {
        assert_eq!(
            status(404, r#"{"error":"not_found","message":"no workspace"}"#).exit_code(),
            6
        );
    }

    #[test]
    fn feature_disabled_is_a_404_and_maps_to_6() {
        // §9 note: feature_disabled arrives as a 404.
        let e = status(
            404,
            r#"{"error":"feature_disabled","message":"promote off"}"#,
        );
        assert_eq!(e.status(), Some(404));
        assert_eq!(e.exit_code(), 6);
    }

    #[test]
    fn bad_request_maps_to_2() {
        assert_eq!(
            status(400, r#"{"error":"bad_request","message":"missing key"}"#).exit_code(),
            2
        );
    }

    #[test]
    fn locked_maps_to_7_and_preserves_holder() {
        let e = status(
            409,
            r#"{"error":"locked","message":"held","who":"rhys","operation":"OperationTypePlan"}"#,
        );
        assert_eq!(e.exit_code(), 7);
        assert_eq!(e.lock_holder(), Some("rhys"));
        assert_eq!(e.message(), Some("held"));
    }

    #[test]
    fn serial_conflict_maps_to_7() {
        assert_eq!(
            status(409, r#"{"error":"serial_conflict","message":"stale"}"#).exit_code(),
            7
        );
    }

    #[test]
    fn payload_too_large_maps_to_1() {
        assert_eq!(
            status(413, r#"{"error":"payload_too_large","message":"10MB cap"}"#).exit_code(),
            1
        );
    }

    #[test]
    fn unmapped_error_string_maps_to_1() {
        assert_eq!(
            status(502, r#"{"error":"upstream_error","message":"boom"}"#).exit_code(),
            1
        );
    }

    #[test]
    fn non_json_body_maps_to_1() {
        let e = status(500, "internal server error");
        assert_eq!(e.error_code(), None);
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn empty_body_maps_to_1() {
        assert_eq!(status(500, "").exit_code(), 1);
    }

    #[test]
    fn message_preserved_even_without_error_code() {
        // A body with only `message` still surfaces the message for display, but
        // maps to the generic code.
        let e = status(400, r#"{"message":"something odd"}"#);
        assert_eq!(e.message(), Some("something odd"));
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn non_status_errors_have_no_code_and_exit_1() {
        let e = ApiError::Body {
            url: "u".into(),
            source: std::io::Error::other("x"),
        };
        assert_eq!(e.error_code(), None);
        assert_eq!(e.exit_code(), 1);
    }
}
