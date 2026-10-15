//! Shared serde models for the Terramantle worker API (SPEC §7).
//!
//! These are the wire types every downstream command (slices 4–7) renders. Field
//! names are grounded against the live worker handlers in
//! `packages/worker/src/routes` — where a shape was uncertain it is modelled
//! loosely (optional + `#[serde(default)]`) and commented. **No model denies
//! unknown fields** (rubric 2): the worker may add columns and the CLI must
//! tolerate them, so every struct simply ignores extras.

use serde::{Deserialize, Serialize};

// ── orgs ─────────────────────────────────────────────────────────────────────

// ── providers: usage rollup ───────────────────────────────────────────────────
// Source: providerFoundry.ts `GET .../providers-usage` (grouped in TS to exactly
// this shape) and `.../providers-usage/:namespace/:type`.

// ── providers: overview (severity rollup for the TRUST column) ─────────────────
// Source: providerScanRead.ts `FoundryOverviewRow`. The handler wraps the array
// as `{ providers: [...] }` — see `ProvidersOverviewResponse`.

// ── providers: available versions (Terraform registry protocol) ────────────────
// Source: providers.ts `GET /v1/providers/:ns/:type/versions` →
// filteredVersionsList → the upstream Terraform protocol `{ versions: [...] }`.

// ── modules ────────────────────────────────────────────────────────────────────
// Source: ModuleService.ts `SearchResponse`, `RegistryVersionSummary`,
// `RegistryVersionResponse`, `RegistryVersionsResponse`.

// ── modules: publish ───────────────────────────────────────────────────────────
// Source: the module-publish endpoint `PUT /v1/modules/{ns}/{name}/{provider}/{version}`
// (§6 step 5) → `{ ok, version_id, storage_key?, status? }`. Returns 201 on success.

// ── state: workspaces + versions ───────────────────────────────────────────────
// Source: StateService.ts `WorkspaceSummary`, `VersionSummary`; state.ts handlers
// wrap them as `{ workspaces: [...] }` / `{ versions: [...] }`.

// ── state: workspace providers (posture polling for lock push) ─────────────────
//
// The types are generated (see the aliases at the foot of this file). This
// behaviour is not, and that split is the point: the generator produces the
// shape, and the judgement about what the shape MEANS stays hand-written and
// tested.

impl VersionScanRollup {
    /// Whether this rollup trips the posture gate.
    ///
    /// `critical || high` only — matching the documented `--at-risk` filter
    /// (§7). `medium`/`low` are deliberately **not** gating: they are carried for
    /// display, and treating them as at-risk would fail nearly every CI run.
    ///
    /// An absent count reads as zero. Note the distinction this method does NOT
    /// make: an absent *rollup* means "not scanned", which is decided by the
    /// caller holding an `Option<VersionScanRollup>` — see `verdict_from_scan`.
    pub fn is_at_risk(&self) -> bool {
        self.critical.unwrap_or(0) > 0 || self.high.unwrap_or(0) > 0
    }
}

// ── state: promote ─────────────────────────────────────────────────────────────
// Source: state.ts promote handler → `{ ok, version_id, serial, source_serial,
// scan_enqueued, ... }`. (The brief said `{version, source_serial}`; the live
// handler returns the richer shape modelled here.)

// ── state: lock ────────────────────────────────────────────────────────────────
// Source: state.ts `GET/DELETE /:workspace/lock`; StateLock in types.ts. GET wraps
// as `{ lock: null | {...} }`. PII fields (who/ip/geo) are member+-gated server-side
// so they are optional here.

// ── state: lock-file push (protocol router) ────────────────────────────────────
// Source: stateProtocol `PUT /state/{org}/{ws}/terraform.lock.hcl` →
// `{ ok, providers_count, warnings }`.

/// Response from the raw lock-file `PUT`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPushResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub providers_count: u64,
    /// Non-fatal parse warnings, echoed from the server.
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ── state: now generated ──────────────────────────────────────────────────────
//
// These were hand-written from the worker's TypeScript, with the endpoint and
// line number in a comment. They are now generated from the spec, and aliased to
// the names this crate already used so call sites did not all have to move.
//
// The generated rows are SUPERSETS of what was modelled by hand: VCS provenance
// (`vcs_repository`, `vcs_ref`, `vcs_sha`, …), policy counts
// (`policy_failing_count` / `policy_total_count`) and the real-vs-internal
// resource split were all being sent and silently dropped, because no model
// denies unknown fields.

/// The `{ lock }` envelope.
pub use crate::generated::models::GetStateLockResponse as LockGetResponse;
/// The `{ orgs }` envelope.
pub use crate::generated::models::ListOrgsResponse as OrgListResponse;
/// The `{ versions }` envelope.
pub use crate::generated::models::ListStateVersionsResponse as VersionListResponse;
/// The `{ workspaces }` envelope.
pub use crate::generated::models::ListStateWorkspacesResponse as WorkspaceListResponse;
/// The org list row, generated from the spec's `OrgSummary`.
///
/// `role` is the spec's `Role` enum rather than a `String`, and the row carries
/// the columns the handler always sent (`name`, `tier`, `is_demo`,
/// `claimed_domain`, …) which the old two-field struct threw away.
pub use crate::generated::models::OrgSummary as OrgMembership;
/// The current lock holder. Was `LockInfo`.
pub use crate::generated::models::StateLock as LockInfo;
/// The promote result. Was `PromoteResponse`.
pub use crate::generated::models::StatePromoteResult as PromoteResponse;
/// One version in a workspace's history. Was `StateVersion`.
pub use crate::generated::models::StateVersionSummary as StateVersion;
/// A state workspace row. Was `Workspace`.
pub use crate::generated::models::StateWorkspaceSummary as Workspace;
/// In-use providers for a workspace, with their per-version scan rollups.
pub use crate::generated::models::{
    VersionScanRollup, WorkspaceProvider, WorkspaceProvidersResponse,
    WorkspaceProvidersResponseLockFile as LockFileMeta,
};

// ── modules: now generated ───────────────────────────────────────────────────
//
// `ModuleDetail` is gone with the versionless path it described: the CLI asked
// for `/v1/modules/{ns}/{name}/{provider}`, which the worker does not route.
// Detail is per version, so the generated `ModuleVersionDetail` replaces it —
// and carries the VCS provenance, findings and policy results the old model did
// not declare.

/// A registry search hit. Was `ModuleSummary`; the severity fields are spelled
/// `*_count` here, as the server sends them.
pub use crate::generated::models::ModuleSearchHit as ModuleSummary;
/// The search response, with its `meta.next_offset` cursor.
pub use crate::generated::models::ModuleSearchResponse;
/// One module version's full detail. Was `ModuleDetail`.
pub use crate::generated::models::ModuleVersionDetail;
/// The Terraform-protocol versions list.
pub use crate::generated::models::ModuleVersionsResponse;
/// A publish acknowledgement: settled, or still scanning.
pub use crate::generated::models::PublishModuleVersionResponse;

// ── providers: now generated ──────────────────────────────────────────────────
//
// `ProviderUsage`, `UsageVersion`, `UsedBy`, `ProviderOverview` and
// `ProvidersOverviewResponse` are gone with the endpoints that fed them: the
// `provider-foundry` usage and overview routes were removed server-side and
// answer 410. The unified catalog replaced all three requests with one, and the
// artefact detail carries the per-version findings and used-by rows the old pair
// could not express.

/// One managed provider with its versions.
pub use crate::generated::models::ProviderArtefactDetail;
/// The org's provider catalog row.
pub use crate::generated::models::ProviderArtefactSummary;
/// Which upstream registry an artefact is mirrored or built from.
pub use crate::generated::models::ProviderUpstream;
/// The registry protocol's provider version list.
pub use crate::generated::models::ProviderVersionsResponse;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a rollup from JSON: the generated type has optional counts and no
    /// `Default`, and going through serde is also the path the real response
    /// takes.
    fn rollup(json: &str) -> VersionScanRollup {
        serde_json::from_str(json).expect("rollup parses")
    }

    /// The shape that mattered. `listWorkspaceProviders` has always sent
    /// `current_scan` / `latest_scan`, but the hand-written model did not declare
    /// them — and because no model denies unknown fields, serde dropped them in
    /// silence. That is what left the posture gate with no severity data to act
    /// on. They now come from the spec, so they cannot go missing again.
    #[test]
    fn workspace_provider_reads_the_scan_rollups() {
        let body = r#"{
            "provider_address":"registry.terraform.io/hashicorp/aws",
            "namespace":"hashicorp","type":"aws","upstream":"terraform.io",
            "current_version":"5.2.0","constraints":"~> 5.0",
            "latest_version":"5.4.1","latest_matching":"5.4.1",
            "current_scan":{"critical":2,"high":3,"medium":7,"low":11},
            "latest_scan":{"critical":0,"high":0,"medium":0,"low":0}
        }"#;
        let p: WorkspaceProvider = serde_json::from_str(body).unwrap();
        assert_eq!(p.namespace, "hashicorp");
        assert_eq!(p.type_, "aws");
        let current = p.current_scan.expect("current_scan present");
        assert_eq!((current.critical, current.high), (Some(2), Some(3)));
        assert!(current.is_at_risk());
        // Present-but-empty is a positive clean result, NOT "unknown".
        assert!(!p.latest_scan.expect("latest_scan present").is_at_risk());
    }

    /// `null` and an omitted key both mean "not scanned", and must stay
    /// distinguishable from an all-zero rollup, which means "scanned and clean".
    #[test]
    fn absent_and_null_rollups_are_both_none() {
        let base =
            r#""namespace":"n","type":"t","upstream":"terraform.io","current_version":"1.0.0""#;

        let p: WorkspaceProvider = serde_json::from_str(&format!("{{{base}}}")).unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        let p: WorkspaceProvider = serde_json::from_str(&format!(
            r#"{{{base},"current_scan":null,"latest_scan":null}}"#
        ))
        .unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        // …whereas an empty object is a real, clean rollup.
        let p: WorkspaceProvider =
            serde_json::from_str(&format!(r#"{{{base},"current_scan":{{}}}}"#)).unwrap();
        let scan = p.current_scan.expect("an empty object is still a rollup");
        assert!(!scan.is_at_risk());
        assert_eq!(scan.critical, None, "absent count, not zero");
    }

    /// Only critical/high gate, matching the documented `--at-risk` filter. A
    /// provider carrying only mediums and lows must not fail a CI push.
    #[test]
    fn only_critical_and_high_are_at_risk() {
        assert!(
            !rollup(r#"{"medium":40,"low":99}"#).is_at_risk(),
            "medium/low must not gate"
        );
        assert!(!rollup("{}").is_at_risk(), "an empty rollup is clean");
        for body in [
            r#"{"critical":1}"#,
            r#"{"high":1}"#,
            r#"{"critical":1,"high":1}"#,
        ] {
            assert!(rollup(body).is_at_risk(), "{body} must gate");
        }
    }

    /// The scanner may add severities; extra keys must not fail the parse.
    #[test]
    fn rollup_tolerates_unknown_severities() {
        let r = rollup(r#"{"critical":1,"high":0,"unknown":5,"negligible":2}"#);
        assert_eq!(r.critical, Some(1));
        assert!(r.is_at_risk());
    }
}
