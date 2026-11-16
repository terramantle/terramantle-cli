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
    /// The `X-Terramantle-Refusal` response HEADER, when the server sent one. Not
    /// part of the JSON body (so never deserialized): a policy refusal on the state
    /// and plan protocol routes carries its reason there as well as in `message`,
    /// and the header is the copy that survives a body some proxy has replaced.
    #[serde(skip)]
    pub refusal: Option<String>,
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
    /// A request URL could not be built from an operation's path template.
    ///
    /// Never a server response — the request was never sent. Either a caller
    /// passed the wrong number of path parameters (a bug) or one was empty,
    /// which is reachable from `--org ''` and must not be silently sent as
    /// `/api/orgs//providers`.
    #[error("{source}")]
    Path {
        #[source]
        source: crate::spec::PathError,
    },
}

impl From<crate::spec::PathError> for ApiError {
    fn from(source: crate::spec::PathError) -> Self {
        ApiError::Path { source }
    }
}

impl ApiError {
    /// Build a `Status` error from a body alone. Tests only: every real response
    /// goes through [`from_status_with_refusal`](Self::from_status_with_refusal).
    #[cfg(test)]
    pub(crate) fn from_status(status: u16, url: String, body: String) -> Self {
        Self::from_status_with_refusal(status, url, body, None)
    }

    /// Build a `Status` error, parsing the body as an `{error, message}` envelope
    /// and keeping the response's `X-Terramantle-Refusal` header beside it. A
    /// public constructor so a caller can test its own error handling against the
    /// shapes the spec documents, without a server. A refusal header with an unparseable body
    /// still yields a parsed envelope (with no `error` code), so the reason is
    /// never dropped just because the body was not the worker's JSON.
    pub fn from_status_with_refusal(
        status: u16,
        url: String,
        body: String,
        refusal: Option<String>,
    ) -> Self {
        let refusal = refusal.filter(|r| !r.trim().is_empty());
        let mut parsed = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .filter(|b| b.error.is_some() || b.message.is_some());
        if let Some(r) = refusal {
            parsed.get_or_insert_with(ErrorBody::default).refusal = Some(r);
        }
        ApiError::Status {
            status,
            url,
            body,
            parsed: parsed.map(Box::new),
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

    /// The server's `X-Terramantle-Refusal` header, when it sent one: the reason a
    /// policy refused the request, in the server's own words.
    pub fn refusal(&self) -> Option<&str> {
        match self {
            ApiError::Status {
                parsed: Some(b), ..
            } => b.refusal.as_deref(),
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
    /// is authoritative; without one, a 401/403 still maps to the auth exit (5) —
    /// not every unauthorized response comes from the worker (gateways, OAuth
    /// issuers, HTML error pages) — and anything else falls back to `1`.
    ///
    /// | `error` | exit |
    /// |---|---|
    /// | `unauthorized`, `invalid_token`, `forbidden` | 5 |
    /// | `approver_role_too_low`, `self_approval` (403: this caller may not approve this plan) | 5 |
    /// | `not_found`, `feature_disabled` (a 404) | 6 |
    /// | `bad_request`, `invalid_tfplan`, `plan_render_mismatch` (400: what was sent is wrong) | 2 |
    /// | `locked`, `serial_conflict`, `lineage_mismatch`, `lineage_required`, `plan_stale` (409) | 7 |
    /// | `policy_blocked` (423: a gate refused, as a posture gate would) | 3 |
    /// | `plan_no_binary` (404) | 6 |
    /// | `human_state_access_blocked` (403) | 5 |
    /// | `payload_too_large` | 1 |
    /// | no code, HTTP 401/403 | 5 |
    /// | unmapped / non-JSON | 1 |
    ///
    /// A [`ApiError::Path`] maps to `2` alongside `bad_request`: the request was
    /// never sent, and the reachable cause is an empty path parameter such as
    /// `--org ''`, which is invalid input rather than a server outcome.
    pub fn exit_code(&self) -> i32 {
        if matches!(self, ApiError::Path { .. }) {
            return 2;
        }
        match self.error_code() {
            Some(
                "unauthorized"
                | "invalid_token"
                | "forbidden"
                | "human_state_access_blocked"
                // The two ways `approvePlanVersion` refuses a caller who IS
                // authenticated and IS an admin: both are "you may not do this",
                // which is what 5 means, not a gate verdict on the plan.
                | "approver_role_too_low"
                | "self_approval",
            ) => 5,
            Some("not_found" | "feature_disabled" | "plan_no_binary") => 6,
            // `plan_render_mismatch`: the render and the binary are not the same
            // plan. Like a malformed plan it is the input that is wrong, and
            // retrying or re-planning against newer state will not fix it.
            Some("bad_request" | "invalid_tfplan" | "plan_render_mismatch") => 2,
            // State-conflict family (§9 exit 7): the artefact does not fit the
            // workspace's current lineage/serial, so the fix is a re-plan, not a retry.
            Some(
                "locked" | "serial_conflict" | "lineage_mismatch" | "lineage_required"
                | "plan_stale",
            ) => 7,
            // A policy refusal is the gate tripping - the same exit `--fail-on-atrisk`
            // uses, so one code means "Terramantle said no" to a pipeline.
            Some("policy_blocked") => 3,
            Some("payload_too_large") => 1,
            Some(_) => 1,
            // No machine-readable code: fall back to the HTTP status for the
            // auth cases, generic 1 otherwise (§9).
            None => match self.status() {
                Some(401) | Some(403) => 5,
                _ => 1,
            },
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

    /// The plan-gate codes, each at the status the spec gives it.
    #[test]
    fn plan_gate_codes_map_to_their_documented_exits() {
        for (http, code, exit) in [
            (400, "plan_render_mismatch", 2),
            (400, "invalid_tfplan", 2),
            (409, "plan_stale", 7),
            (409, "lineage_mismatch", 7),
            (423, "policy_blocked", 3),
            (404, "plan_no_binary", 6),
            (403, "approver_role_too_low", 5),
            (403, "self_approval", 5),
            (403, "human_state_access_blocked", 5),
        ] {
            let e = status(http, &format!(r#"{{"error":"{code}","message":"m"}}"#));
            assert_eq!(e.exit_code(), exit, "{code}");
            assert_eq!(e.error_code(), Some(code));
        }
    }

    #[test]
    fn refusal_header_is_kept_beside_the_body() {
        let e = ApiError::from_status_with_refusal(
            423,
            "u".into(),
            r#"{"error":"policy_blocked","message":"blocked: 3 destroys"}"#.into(),
            Some("blocked: 3 destroys".into()),
        );
        assert_eq!(e.refusal(), Some("blocked: 3 destroys"));
        assert_eq!(e.error_code(), Some("policy_blocked"));
        assert_eq!(e.exit_code(), 3);
        // The body-only constructor leaves it absent.
        assert_eq!(status(423, r#"{"error":"policy_blocked"}"#).refusal(), None);
    }

    /// A proxy that swallowed the JSON body must not also swallow the reason,
    /// and a body with no code still falls back to the status for its exit.
    #[test]
    fn refusal_header_survives_a_non_json_body() {
        let e = ApiError::from_status_with_refusal(
            403,
            "u".into(),
            "<html>forbidden</html>".into(),
            Some("needs write".into()),
        );
        assert_eq!(e.refusal(), Some("needs write"));
        assert_eq!(e.error_code(), None);
        assert_eq!(e.exit_code(), 5);
        // An empty header is no header.
        let blank =
            ApiError::from_status_with_refusal(500, "u".into(), "".into(), Some(" ".into()));
        assert_eq!(blank.refusal(), None);
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
    fn codeless_401_and_403_fall_back_to_status_exit_5() {
        // Not everything that 401s speaks the worker envelope: OAuth issuers,
        // gateways, HTML error pages.
        assert_eq!(status(401, "<html>unauthorized</html>").exit_code(), 5);
        assert_eq!(status(403, "").exit_code(), 5);
        // But an unmapped explicit code still wins over the status.
        assert_eq!(status(401, r#"{"error":"weird_code"}"#).exit_code(), 1);
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

    /// A path that could not be built is invalid input, not a server failure:
    /// nothing was sent, so there is no status and no `error` code to map.
    #[test]
    fn path_errors_exit_2_and_carry_no_server_code() {
        let e = ApiError::from(crate::spec::PathError::EmptyParam {
            op: "listOrgProviders",
            name: "orgId",
        });
        assert_eq!(e.exit_code(), 2);
        assert_eq!(e.status(), None);
        assert_eq!(e.error_code(), None);
        assert!(e.to_string().contains("orgId"));
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
