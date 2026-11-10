//! Plan endpoints (`plan push/pull/ls/show`).
//!
//! The plan surface hangs off the state key: `/state/{org}/{key}/plan` and
//! `…/plan/latest` | `…/plan/{planId}`. The key is a router wildcard, so these
//! three are in the codegen's `WILDCARD_PARAMS` list and the key is joined verbatim.
//!
//! A push holds the connection open while the worker runs its synchronous policy
//! gate and (for a binary) reads the plan's embedded prior state, so both transfer
//! calls carry a whole-request budget of [`PLAN_TRANSFER_TIMEOUT`] rather than the
//! agent's 30s per-phase defaults.

use std::time::Duration;

use crate::client::MultipartPart;
use crate::error::ApiError;
use crate::generated::models::{
    GetPlanVersionResponse, ListPlanVersionsResponse, PlanPushResult, PlanVersionSummary,
};
use crate::ops;
use crate::state::VERSIONS_PAGE_MAX;
use crate::Client;

/// Whole-request budget for a plan push or pull: a 50MB binary over a slow CI
/// uplink plus the gate (OPA alone may take 3s before failing closed).
pub const PLAN_TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);

/// Headers a push may carry. Prior serial/lineage are only read by the server
/// for a RENDER-ONLY push; with a binary it reads them from the plan itself.
#[derive(Debug, Clone, Default)]
pub struct PlanPushOptions<'a> {
    /// `X-Plan-Ref`: PR / branch / SHA discriminator. `None` = mainline.
    pub plan_ref: Option<&'a str>,
    pub prior_serial: Option<i64>,
    pub prior_lineage: Option<&'a str>,
    /// `X-Plan-Allow-Stale: 1` — keep a binary whose prior serial is not current.
    pub allow_stale: bool,
}

/// Which binary a pull addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSelector<'a> {
    /// The newest version for a ref (`None` = mainline).
    Latest { plan_ref: Option<&'a str> },
    /// One version by `plan_versions.id`.
    Id(&'a str),
}

/// A pulled binary plan plus the identity the server stamped on it.
#[derive(Debug, Clone)]
pub struct PlanPull {
    pub bytes: Vec<u8>,
    pub plan_id: Option<String>,
    pub attestation_hash: Option<String>,
    /// What the caller MUST verify the bytes against before using them.
    pub tfplan_sha256: Option<String>,
    pub prior_serial: Option<i64>,
    pub prior_lineage: Option<String>,
    /// The server's staleness read at pull time (only ever true with `allow_stale`).
    pub stale: bool,
}

impl Client {
    /// `POST /state/{org}/{key}/plan` — push a `show -json` render, with the
    /// binary `.tfplan` when the caller has one. JSON body for a render-only push;
    /// multipart (`render` + `tfplan` parts) otherwise. The response carries the
    /// synchronous gate verdict.
    pub fn plan_push(
        &self,
        org: &str,
        key: &str,
        render: &str,
        tfplan: Option<&[u8]>,
        opts: &PlanPushOptions<'_>,
    ) -> Result<PlanPushResult, ApiError> {
        let path = ops::PUSH_PLAN.path(&[org, key])?;
        let serial = opts.prior_serial.map(|s| s.to_string());
        let mut headers: Vec<(&str, &str)> = Vec::new();
        if let Some(r) = opts.plan_ref {
            headers.push(("X-Plan-Ref", r));
        }
        if let Some(s) = serial.as_deref() {
            headers.push(("X-Plan-Prior-Serial", s));
        }
        if let Some(l) = opts.prior_lineage {
            headers.push(("X-Plan-Prior-Lineage", l));
        }
        if opts.allow_stale {
            headers.push(("X-Plan-Allow-Stale", "1"));
        }
        match tfplan {
            None => self.http().post_bytes(
                &path,
                render.as_bytes(),
                "application/json",
                &headers,
                PLAN_TRANSFER_TIMEOUT,
            ),
            Some(bytes) => {
                let parts = [
                    MultipartPart {
                        name: "render",
                        filename: Some("plan.json"),
                        content_type: "application/json",
                        data: render.as_bytes(),
                    },
                    MultipartPart {
                        name: "tfplan",
                        filename: Some("plan.tfplan"),
                        content_type: "application/octet-stream",
                        data: bytes,
                    },
                ];
                self.http()
                    .post_multipart(&path, &parts, &headers, PLAN_TRANSFER_TIMEOUT)
            }
        }
    }

    /// `GET /state/{org}/{key}/plan/latest?ref=` or `…/plan/{planId}` — the binary
    /// plan, byte for byte, with the server's identity headers. The caller verifies
    /// `tfplan_sha256` over `bytes` before writing them anywhere a `terraform apply`
    /// could pick them up.
    pub fn plan_pull(
        &self,
        org: &str,
        key: &str,
        selector: PlanSelector<'_>,
        allow_stale: bool,
    ) -> Result<PlanPull, ApiError> {
        let (path, mut query): (String, Vec<(&str, String)>) = match selector {
            PlanSelector::Latest { plan_ref } => {
                let mut q = Vec::new();
                if let Some(r) = plan_ref {
                    q.push(("ref", r.to_string()));
                }
                (ops::GET_LATEST_PLAN_BINARY.path(&[org, key])?, q)
            }
            PlanSelector::Id(id) => (ops::GET_PLAN_BINARY.path(&[org, key, id])?, Vec::new()),
        };
        if allow_stale {
            query.push(("allow_stale", "1".to_string()));
        }
        let raw = self.http().get_raw(&path, &query, PLAN_TRANSFER_TIMEOUT)?;
        let header = |n: &str| raw.header(n).map(str::to_string);
        Ok(PlanPull {
            plan_id: header("X-Terramantle-Plan-Id"),
            attestation_hash: header("X-Terramantle-Attestation-Hash"),
            tfplan_sha256: header("X-Terramantle-Tfplan-Sha256").filter(|s| !s.is_empty()),
            prior_serial: header("X-Terramantle-Prior-Serial").and_then(|s| s.parse().ok()),
            prior_lineage: header("X-Terramantle-Prior-Lineage"),
            stale: header("X-Terramantle-Plan-Stale").as_deref() == Some("true"),
            bytes: raw.body,
        })
    }

    /// `GET /api/v1/{org}/plans/{ws}/versions?limit=&offset=` — one page of a
    /// workspace's plan history, newest first. Unwraps the `{ versions }` envelope.
    pub fn plan_versions(
        &self,
        org: &str,
        workspace: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<PlanVersionSummary>, ApiError> {
        let path = ops::LIST_PLAN_VERSIONS.path(&[org, workspace])?;
        let query = [
            ("limit", limit.min(VERSIONS_PAGE_MAX).to_string()),
            ("offset", offset.to_string()),
        ];
        let resp: ListPlanVersionsResponse = self.http().get_json_query(&path, &query)?;
        Ok(resp.versions)
    }

    /// `GET /api/v1/{org}/plans/{ws}/versions/{id}` — one version's summary plus
    /// its redacted findings.
    pub fn plan_version(
        &self,
        org: &str,
        workspace: &str,
        version_id: &str,
    ) -> Result<GetPlanVersionResponse, ApiError> {
        let path = ops::GET_PLAN_VERSION.path(&[org, workspace, version_id])?;
        self.http().get_json(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three plan paths keep the wildcard key's slashes and encode the id.
    #[test]
    fn plan_paths_keep_the_key_verbatim() {
        assert_eq!(
            ops::PUSH_PLAN
                .path(&["acme", "stacks/data/platform"])
                .unwrap(),
            "/state/acme/stacks/data/platform/plan"
        );
        assert_eq!(
            ops::GET_LATEST_PLAN_BINARY.path(&["acme", "prod"]).unwrap(),
            "/state/acme/prod/plan/latest"
        );
        assert_eq!(
            ops::GET_PLAN_BINARY
                .path(&["acme", "a/b", "0f8fad5b-d9cb-469f-a165-70867728950e"])
                .unwrap(),
            "/state/acme/a/b/plan/0f8fad5b-d9cb-469f-a165-70867728950e"
        );
    }

    #[test]
    fn plan_pull_parses_the_identity_headers() {
        // Exercise the header folding without a server: build the struct the way
        // plan_pull does from a RawResponse.
        let raw = crate::RawResponse {
            status: 200,
            headers: vec![
                ("x-terramantle-plan-id".into(), "p1".into()),
                ("x-terramantle-tfplan-sha256".into(), "".into()),
                ("x-terramantle-prior-serial".into(), "7".into()),
                ("x-terramantle-plan-stale".into(), "true".into()),
            ],
            body: vec![1, 2, 3],
        };
        let header = |n: &str| raw.header(n).map(str::to_string);
        let pull = PlanPull {
            plan_id: header("X-Terramantle-Plan-Id"),
            attestation_hash: header("X-Terramantle-Attestation-Hash"),
            tfplan_sha256: header("X-Terramantle-Tfplan-Sha256").filter(|s| !s.is_empty()),
            prior_serial: header("X-Terramantle-Prior-Serial").and_then(|s| s.parse().ok()),
            prior_lineage: header("X-Terramantle-Prior-Lineage"),
            stale: header("X-Terramantle-Plan-Stale").as_deref() == Some("true"),
            bytes: raw.body.clone(),
        };
        assert_eq!(pull.plan_id.as_deref(), Some("p1"));
        assert_eq!(
            pull.tfplan_sha256, None,
            "an empty sha header reads as absent"
        );
        assert_eq!(pull.prior_serial, Some(7));
        assert!(pull.stale);
        assert_eq!(pull.bytes, vec![1, 2, 3]);
    }
}
