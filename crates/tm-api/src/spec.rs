//! The spec-derived operation model: what an endpoint IS, independent of how a
//! request is sent.
//!
//! Hand-written; `generated/operations.rs` is the part built from the spec. The
//! split is deliberate — the generator produces data, and the behaviour that
//! data drives (path building, encoding) is reviewable code with its own tests.
//!
//! This layer sits ABOVE [`crate::HttpClient`] and knows nothing about it, so
//! the blocking `ureq` transport, the refresh-on-401 retry and the body-driven
//! exit-code map are all untouched by codegen.

use std::fmt;

/// How a path parameter is substituted into a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// One path segment: percent-encoded, so a value containing `/`, `?` or `#`
    /// cannot escape its segment and rewrite the route.
    Segment,
    /// The rest of the path, joined verbatim.
    ///
    /// Terraform's state and plan routers match a WILDCARD, so the key may
    /// legitimately contain slashes (`prod/backend`). The spec says as much on
    /// `components.parameters.StateKey` and notes that "OpenAPI path templating
    /// cannot express" it. Encoding such a value would send
    /// `/state/acme/prod%2F.terraform.lock.hcl` and get a 404, which is why this
    /// variant exists and why `lock push` depends on it.
    RawPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
    Patch,
    Head,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Post => "POST",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
            Method::Head => "HEAD",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One API operation, as the spec describes it.
#[derive(Debug, Clone, Copy)]
pub struct Op {
    /// The spec's `operationId` — the stable identity this registry is keyed on.
    pub id: &'static str,
    pub method: Method,
    /// The path template, copied from the spec verbatim (e.g.
    /// `/api/orgs/{orgId}/providers`).
    pub template: &'static str,
    /// Path parameters in TEMPLATE order, which is the order [`Op::path`] takes
    /// its arguments in.
    pub params: &'static [(&'static str, ParamKind)],
    /// Declared query parameter names, for the contract test to check against.
    pub query: &'static [&'static str],
    /// The success status the spec documents.
    pub success: u16,
}

/// Why a path could not be built. Always a programming error, never a server
/// response, so it is separate from `ApiError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Wrong number of arguments for this operation's template.
    Arity {
        op: &'static str,
        expected: usize,
        got: usize,
    },
    /// A path parameter was empty, which would collapse two segments into one
    /// and silently address a different route.
    EmptyParam {
        op: &'static str,
        name: &'static str,
    },
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Arity { op, expected, got } => {
                write!(f, "{op} takes {expected} path parameter(s), got {got}")
            }
            PathError::EmptyParam { op, name } => {
                write!(f, "{op} path parameter `{name}` was empty")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// Percent-encode one path segment, keeping the RFC 3986 unreserved set.
///
/// Deliberately strict: everything outside `A-Za-z0-9-_.~` is escaped, so `/`,
/// `?`, `#` and `%` in an org slug or workspace name cannot alter the route.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Op {
    /// Build this operation's path, substituting `args` in template order.
    ///
    /// [`ParamKind::Segment`] values are percent-encoded; [`ParamKind::RawPath`]
    /// values are joined verbatim. Arity is checked, so a caller that forgets an
    /// argument gets a named error rather than a URL with a literal `{orgId}` in
    /// it.
    pub fn path(&self, args: &[&str]) -> Result<String, PathError> {
        if args.len() != self.params.len() {
            return Err(PathError::Arity {
                op: self.id,
                expected: self.params.len(),
                got: args.len(),
            });
        }
        let mut path = self.template.to_string();
        for ((name, kind), value) in self.params.iter().zip(args) {
            if value.is_empty() {
                return Err(PathError::EmptyParam { op: self.id, name });
            }
            let replacement = match kind {
                ParamKind::Segment => encode_segment(value),
                ParamKind::RawPath => (*value).to_string(),
            };
            path = path.replace(&format!("{{{name}}}"), &replacement);
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: Op = Op {
        id: "listOrgProviders",
        method: Method::Get,
        template: "/api/orgs/{orgId}/providers",
        params: &[("orgId", ParamKind::Segment)],
        query: &[],
        success: 200,
    };

    const UPLOAD: Op = Op {
        id: "uploadLockFile",
        method: Method::Put,
        template: "/state/{org}/{key}",
        params: &[("org", ParamKind::Segment), ("key", ParamKind::RawPath)],
        query: &[],
        success: 200,
    };

    #[test]
    fn builds_a_simple_path() {
        assert_eq!(
            LIST.path(&["acme-demo"]).unwrap(),
            "/api/orgs/acme-demo/providers"
        );
    }

    /// The template is the spec's, so the trailing slash that broke `state ls`
    /// cannot be reintroduced by a caller.
    #[test]
    fn never_emits_a_trailing_slash() {
        let p = LIST.path(&["acme"]).unwrap();
        assert!(!p.ends_with('/'), "{p}");
    }

    /// A wildcard key keeps its slashes. Encoding them would send
    /// `prod%2F.terraform.lock.hcl` and 404 — this is the `lock push` contract.
    #[test]
    fn wildcard_param_keeps_its_slashes() {
        assert_eq!(
            UPLOAD
                .path(&["acme", "prod/backend/.terraform.lock.hcl"])
                .unwrap(),
            "/state/acme/prod/backend/.terraform.lock.hcl"
        );
    }

    /// …while an ordinary segment is encoded, so it cannot escape its segment.
    #[test]
    fn segment_param_is_encoded() {
        assert_eq!(
            LIST.path(&["acme/../other"]).unwrap(),
            "/api/orgs/acme%2F..%2Fother/providers"
        );
        assert_eq!(
            LIST.path(&["a b&c=d"]).unwrap(),
            "/api/orgs/a%20b%26c%3Dd/providers"
        );
    }

    #[test]
    fn arity_mismatch_is_an_error_not_a_literal_brace() {
        let err = UPLOAD.path(&["acme"]).unwrap_err();
        assert_eq!(
            err,
            PathError::Arity {
                op: "uploadLockFile",
                expected: 2,
                got: 1
            }
        );
        // The failure mode this prevents: a path still containing its template.
        assert!(!format!("{err}").contains('{'));
    }

    /// An empty value would collapse `/orgs//providers` and address a different
    /// route, so it is refused rather than silently sent.
    #[test]
    fn empty_param_is_refused() {
        assert_eq!(
            LIST.path(&[""]).unwrap_err(),
            PathError::EmptyParam {
                op: "listOrgProviders",
                name: "orgId"
            }
        );
    }

    #[test]
    fn method_renders_as_the_http_token() {
        assert_eq!(Method::Delete.to_string(), "DELETE");
        assert_eq!(Method::Get.as_str(), "GET");
    }
}
