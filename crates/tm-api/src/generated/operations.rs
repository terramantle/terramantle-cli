//! Every operation the Terramantle API serves, as a constant keyed on its
//! `operationId`.
//!
//! Endpoint paths are NOT written by hand anywhere in this crate. A path that
//! the spec does not describe has no constant here, so calling it does not
//! compile; a path the spec describes cannot acquire a typo or a trailing
//! slash, because the template below is copied from the spec verbatim.
use crate::spec::{Method, Op, ParamKind};
/// Operations the spec marks as removed (their only response is `410`).
/// Listed for the contract test; deliberately NOT given constants, so a call
/// site that still refers to one fails to compile rather than 410 at runtime.
pub const TOMBSTONED_OPERATION_IDS: &[&str] = &[
    "betaUpgrade",
    "removedProviderFoundryRoute",
    "removedProviderMirrorRoute",
];
/// `POST /api/orgs/{orgId}/members`
///
/// Tag: Orgs & Members. Success: `200`.
pub const ADD_ORG_MEMBER: Op = Op {
    id: "addOrgMember",
    method: Method::Post,
    template: "/api/orgs/{orgId}/members",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers`
///
/// Tag: Providers — Management. Success: `201`.
pub const ADD_ORG_PROVIDER: Op = Op {
    id: "addOrgProvider",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 201,
};
/// `POST /api/orgs/{orgId}/policies/baseline`
///
/// Tag: Policies. Success: `201`.
pub const APPLY_POLICY_BASELINE: Op = Op {
    id: "applyPolicyBaseline",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies/baseline",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 201,
};
/// `POST /api/v1/{orgId}/plans/{workspace}/versions/{versionId}/approve`
///
/// Tag: Plans. Success: `200`.
pub const APPROVE_PLAN_VERSION: Op = Op {
    id: "approvePlanVersion",
    method: Method::Post,
    template: "/api/v1/{orgId}/plans/{workspace}/versions/{versionId}/approve",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/policy-attest`
///
/// Tag: Provider Builds. Success: `200`.
pub const ATTEST_PROVIDER_POLICY_BLOCK: Op = Op {
    id: "attestProviderPolicyBlock",
    method: Method::Post,
    template:
        "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/policy-attest",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/v1/{orgId}/state/{workspace}/policy/attest`
///
/// Tag: State Management. Success: `200`.
pub const ATTEST_STATE_POLICY_BLOCK: Op = Op {
    id: "attestStatePolicyBlock",
    method: Method::Post,
    template: "/api/v1/{orgId}/state/{workspace}/policy/attest",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/auto-patch`
///
/// Tag: Provider Builds. Success: `200`.
pub const AUTO_PATCH_PROVIDER_VERSION: Op = Op {
    id: "autoPatchProviderVersion",
    method: Method::Post,
    template:
        "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/auto-patch",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /v1/modules/{namespace}/{name}/{provider}/{version}/block`
///
/// Tag: Registry — Modules. Success: `200`.
pub const BLOCK_MODULE_VERSION: Op = Op {
    id: "blockModuleVersion",
    method: Method::Post,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/block",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/blocklist`
///
/// Tag: Provider Builds. Success: `200`.
pub const BLOCKLIST_PROVIDER_VERSION: Op = Op {
    id: "blocklistProviderVersion",
    method: Method::Post,
    template:
        "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/blocklist",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/build`
///
/// Tag: Provider Builds. Success: `200`.
pub const BUILD_PROVIDER_VERSION: Op = Op {
    id: "buildProviderVersion",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/build",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/claim-domain`
///
/// Tag: Orgs & Members. Success: `200`.
pub const CLAIM_ORG_DOMAIN: Op = Op {
    id: "claimOrgDomain",
    method: Method::Post,
    template: "/api/orgs/{orgId}/claim-domain",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/billing/portal`
///
/// Tag: Billing. Success: `200`.
pub const CREATE_BILLING_PORTAL_SESSION: Op = Op {
    id: "createBillingPortalSession",
    method: Method::Post,
    template: "/api/orgs/{orgId}/billing/portal",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/billing/checkout`
///
/// Tag: Billing. Success: `200`.
pub const CREATE_CHECKOUT_SESSION: Op = Op {
    id: "createCheckoutSession",
    method: Method::Post,
    template: "/api/orgs/{orgId}/billing/checkout",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs`
///
/// Tag: Orgs & Members. Success: `201`.
pub const CREATE_ORG: Op = Op {
    id: "createOrg",
    method: Method::Post,
    template: "/api/orgs",
    params: &[],
    query: &[],
    success: 201,
};
/// `POST /api/orgs/{orgId}/tokens`
///
/// Tag: Tokens. Success: `201`.
pub const CREATE_ORG_TOKEN: Op = Op {
    id: "createOrgToken",
    method: Method::Post,
    template: "/api/orgs/{orgId}/tokens",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 201,
};
/// `POST /api/orgs/{orgId}/policies`
///
/// Tag: Policies. Success: `201`.
pub const CREATE_POLICY: Op = Op {
    id: "createPolicy",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 201,
};
/// `POST /api/orgs/{orgId}/vcs-trust`
///
/// Tag: VCS Trust. Success: `201`.
pub const CREATE_VCS_TRUST: Op = Op {
    id: "createVcsTrust",
    method: Method::Post,
    template: "/api/orgs/{orgId}/vcs-trust",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 201,
};
/// `DELETE /api/namespaces/{namespace}/modules/{name}`
///
/// Tag: Registry — Modules. Success: `200`.
pub const DELETE_NAMESPACE_MODULE: Op = Op {
    id: "deleteNamespaceModule",
    method: Method::Delete,
    template: "/api/namespaces/{namespace}/modules/{name}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}`
///
/// Tag: Orgs & Members. Success: `200`.
pub const DELETE_ORG: Op = Op {
    id: "deleteOrg",
    method: Method::Delete,
    template: "/api/orgs/{orgId}",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}`
///
/// Tag: Providers — Management. Success: `200`.
pub const DELETE_ORG_PROVIDER: Op = Op {
    id: "deleteOrgProvider",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/policies/{id}`
///
/// Tag: Policies. Success: `200`.
pub const DELETE_POLICY: Op = Op {
    id: "deletePolicy",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/policies/{id}",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/v1/{orgId}/state/{workspace}/versions/{versionId}/resources/delete`
///
/// Tag: State Management. Success: `200`.
pub const DELETE_STATE_RESOURCES: Op = Op {
    id: "deleteStateResources",
    method: Method::Post,
    template: "/api/v1/{orgId}/state/{workspace}/versions/{versionId}/resources/delete",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/v1/{orgId}/state/{workspace}`
///
/// Tag: State Management. Success: `200`.
pub const DELETE_STATE_WORKSPACE: Op = Op {
    id: "deleteStateWorkspace",
    method: Method::Delete,
    template: "/api/v1/{orgId}/state/{workspace}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/vcs-trust/{trustId}`
///
/// Tag: VCS Trust. Success: `200`.
pub const DELETE_VCS_TRUST: Op = Op {
    id: "deleteVcsTrust",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/vcs-trust/{trustId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("trustId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /v1/modules/{namespace}/{name}/{provider}/deprecate`
///
/// Tag: Registry — Modules. Success: `200`.
pub const DEPRECATE_MODULE: Op = Op {
    id: "deprecateModule",
    method: Method::Put,
    template: "/v1/modules/{namespace}/{name}/{provider}/deprecate",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /v1/modules/{namespace}/{name}/{provider}/{version}/deprecate`
///
/// Tag: Registry — Modules. Success: `200`.
pub const DEPRECATE_MODULE_VERSION: Op = Op {
    id: "deprecateModuleVersion",
    method: Method::Put,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/deprecate",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/{version}/download`
///
/// Tag: Registry — Modules. Success: `204`.
pub const DOWNLOAD_MODULE_VERSION: Op = Op {
    id: "downloadModuleVersion",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/download",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 204,
};
/// `POST /api/orgs/{orgId}/policies/instance/force`
///
/// Tag: Policies. Success: `200`.
pub const FORCE_INSTANCE_POLICIES: Op = Op {
    id: "forceInstancePolicies",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies/instance/force",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/policies/{id}/force-inheritance`
///
/// Tag: Policies. Success: `200`.
pub const FORCE_POLICY_INHERITANCE: Op = Op {
    id: "forcePolicyInheritance",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies/{id}/force-inheritance",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /v1/modules/{namespace}/{name}/{provider}/{version}/unblock`
///
/// Tag: Registry — Modules. Success: `200`.
pub const FORCE_UNBLOCK_MODULE_VERSION: Op = Op {
    id: "forceUnblockModuleVersion",
    method: Method::Post,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/unblock",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/v1/{orgId}/state/{workspace}/lock`
///
/// Tag: State Management. Success: `200`.
pub const FORCE_UNLOCK_STATE: Op = Op {
    id: "forceUnlockState",
    method: Method::Delete,
    template: "/api/v1/{orgId}/state/{workspace}/lock",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/policy-schema/{artefact}`
///
/// Tag: Public. Success: `200`.
pub const GET_ARTEFACT_POLICY_INPUT_SCHEMA: Op = Op {
    id: "getArtefactPolicyInputSchema",
    method: Method::Get,
    template: "/api/policy-schema/{artefact}",
    params: &[("artefact", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /asyncapi.json`
///
/// Tag: Meta. Success: `200`.
pub const GET_ASYNC_API_JSON: Op = Op {
    id: "getAsyncApiJson",
    method: Method::Get,
    template: "/asyncapi.json",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /asyncapi.yaml`
///
/// Tag: Meta. Success: `200`.
pub const GET_ASYNC_API_YAML: Op = Op {
    id: "getAsyncApiYaml",
    method: Method::Get,
    template: "/asyncapi.yaml",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/billing`
///
/// Tag: Billing. Success: `200`.
pub const GET_BILLING: Op = Op {
    id: "getBilling",
    method: Method::Get,
    template: "/api/orgs/{orgId}/billing",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/billing/limits`
///
/// Tag: Billing. Success: `200`.
pub const GET_BILLING_LIMITS: Op = Op {
    id: "getBillingLimits",
    method: Method::Get,
    template: "/api/orgs/{orgId}/billing/limits",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /.well-known/terramantle-cli.json`
///
/// Tag: Discovery. Success: `200`.
pub const GET_CLI_DISCOVERY: Op = Op {
    id: "getCliDiscovery",
    method: Method::Get,
    template: "/.well-known/terramantle-cli.json",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/consumption-block`
///
/// Tag: Policies. Success: `200`.
pub const GET_CONSUMPTION_BLOCK: Op = Op {
    id: "getConsumptionBlock",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/consumption-block",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/auth/me`
///
/// Tag: Orgs & Members. Success: `200`.
pub const GET_CURRENT_USER: Op = Op {
    id: "getCurrentUser",
    method: Method::Get,
    template: "/api/auth/me",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /docs`
///
/// Tag: Meta. Success: `200`.
pub const GET_DOCS: Op = Op {
    id: "getDocs",
    method: Method::Get,
    template: "/docs",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /docs/oauth2-redirect`
///
/// Tag: Meta. Success: `200`.
pub const GET_DOCS_OAUTH_REDIRECT: Op = Op {
    id: "getDocsOauthRedirect",
    method: Method::Get,
    template: "/docs/oauth2-redirect",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /echo-status/{code}`
///
/// Tag: Meta. Success: `200`.
pub const GET_ECHO_STATUS: Op = Op {
    id: "getEchoStatus",
    method: Method::Get,
    template: "/echo-status/{code}",
    params: &[("code", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /health`
///
/// Tag: Meta. Success: `200`.
pub const GET_HEALTH: Op = Op {
    id: "getHealth",
    method: Method::Get,
    template: "/health",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/{version}/archive.tar.gz`
///
/// Tag: Registry — Modules. Success: `200`.
pub const GET_MODULE_ARCHIVE: Op = Op {
    id: "getModuleArchive",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/archive.tar.gz",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/{version}`
///
/// Tag: Registry — Modules. Success: `200`.
pub const GET_MODULE_VERSION: Op = Op {
    id: "getModuleVersion",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/{version}/graph`
///
/// Tag: Registry — Modules. Success: `200`.
pub const GET_MODULE_VERSION_GRAPH: Op = Op {
    id: "getModuleVersionGraph",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/graph",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/{version}/status`
///
/// Tag: Registry — Modules. Success: `200`.
pub const GET_MODULE_VERSION_STATUS: Op = Op {
    id: "getModuleVersionStatus",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/status",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /openapi.json`
///
/// Tag: Meta. Success: `200`.
pub const GET_OPEN_API_JSON: Op = Op {
    id: "getOpenApiJson",
    method: Method::Get,
    template: "/openapi.json",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /openapi.yaml`
///
/// Tag: Meta. Success: `200`.
pub const GET_OPEN_API_YAML: Op = Op {
    id: "getOpenApiYaml",
    method: Method::Get,
    template: "/openapi.yaml",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/analytics`
///
/// Tag: Analytics. Success: `200`.
pub const GET_ORG_ANALYTICS: Op = Op {
    id: "getOrgAnalytics",
    method: Method::Get,
    template: "/api/orgs/{orgId}/analytics",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/graph`
///
/// Tag: Analytics. Success: `200`.
pub const GET_ORG_DEPENDENCY_GRAPH: Op = Op {
    id: "getOrgDependencyGraph",
    method: Method::Get,
    template: "/api/v1/{orgId}/graph",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}`
///
/// Tag: Providers — Management. Success: `200`.
pub const GET_ORG_PROVIDER: Op = Op {
    id: "getOrgProvider",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/download/{os}/{arch}`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_PROVIDER_DOWNLOAD: Op = Op {
    id: "getOrgProviderDownload",
    method: Method::Get,
    template: "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/download/{os}/{arch}",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("os", ParamKind::Segment),
        ("arch", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_PROVIDER_SHASUMS: Op = Op {
    id: "getOrgProviderShasums",
    method: Method::Get,
    template: "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS.sig`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_PROVIDER_SHASUMS_SIG: Op = Op {
    id: "getOrgProviderShasumsSig",
    method: Method::Get,
    template:
        "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS.sig",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/{filename}`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_PROVIDER_ZIP: Op = Op {
    id: "getOrgProviderZip",
    method: Method::Get,
    template: "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/{upstream}/{filename}",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("filename", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_SELF_BUILT_SHASUMS: Op = Op {
    id: "getOrgSelfBuiltShasums",
    method: Method::Get,
    template:
        "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS.sig`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_SELF_BUILT_SHASUMS_SIG: Op = Op {
    id: "getOrgSelfBuiltShasumsSig",
    method: Method::Get,
    template: "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS.sig",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/{filename}`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_ORG_SELF_BUILT_ZIP: Op = Op {
    id: "getOrgSelfBuiltZip",
    method: Method::Get,
    template:
        "/v1/providers/{orgSlug}/{namespace}/{type}/{version}/files/self_built/{lineage}/{filename}",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
        ("filename", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/settings`
///
/// Tag: Orgs & Members. Success: `200`.
pub const GET_ORG_SETTINGS: Op = Op {
    id: "getOrgSettings",
    method: Method::Get,
    template: "/api/orgs/{orgId}/settings",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/attestation`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_ATTESTATION_BY_HASH: Op = Op {
    id: "getPlanAttestationByHash",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/attestation",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &["hash"],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/versions/{versionId}/attestation`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_ATTESTATION_BY_VERSION: Op = Op {
    id: "getPlanAttestationByVersion",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/versions/{versionId}/attestation",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/policy-config`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_POLICY_CONFIG: Op = Op {
    id: "getPlanPolicyConfig",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/policy-config",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/replacements-by-provider`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_REPLACEMENTS_BY_PROVIDER: Op = Op {
    id: "getPlanReplacementsByProvider",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/replacements-by-provider",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/versions/{versionId}`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_VERSION: Op = Op {
    id: "getPlanVersion",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/versions/{versionId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/versions/{versionId}/policy`
///
/// Tag: Plans. Success: `200`.
pub const GET_PLAN_VERSION_POLICY_RESULTS: Op = Op {
    id: "getPlanVersionPolicyResults",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/versions/{versionId}/policy",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/{id}`
///
/// Tag: Policies. Success: `200`.
pub const GET_POLICY: Op = Op {
    id: "getPolicy",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/{id}",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/coverage`
///
/// Tag: Policies. Success: `200`.
pub const GET_POLICY_COVERAGE: Op = Op {
    id: "getPolicyCoverage",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/coverage",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/policy-schema`
///
/// Tag: Public. Success: `200`.
pub const GET_POLICY_INPUT_SCHEMA: Op = Op {
    id: "getPolicyInputSchema",
    method: Method::Get,
    template: "/api/policy-schema",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/settings`
///
/// Tag: Policies. Success: `200`.
pub const GET_POLICY_SETTINGS: Op = Op {
    id: "getPolicySettings",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/settings",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /.well-known/terramantle-portal.json`
///
/// Tag: Discovery. Success: `200`.
pub const GET_PORTAL_DISCOVERY: Op = Op {
    id: "getPortalDiscovery",
    method: Method::Get,
    template: "/.well-known/terramantle-portal.json",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/flow`
///
/// Tag: Provider Builds. Success: `200`.
pub const GET_PROVIDER_BUILD_FLOW: Op = Op {
    id: "getProviderBuildFlow",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/flow",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &["detail"],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/consume`
///
/// Tag: Providers — Management. Success: `200`.
pub const GET_PROVIDER_CONSUME_INSTRUCTIONS: Op = Op {
    id: "getProviderConsumeInstructions",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/consume",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/download/{os}/{arch}`
///
/// Tag: Registry — Providers. Success: `200`.
#[deprecated(
    note = "the spec marks `getProviderDownload` deprecated; see its description for the replacement"
)]
pub const GET_PROVIDER_DOWNLOAD: Op = Op {
    id: "getProviderDownload",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/download/{os}/{arch}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("os", ParamKind::Segment),
        ("arch", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/comparison`
///
/// Tag: Providers — Management. Success: `200`.
pub const GET_PROVIDER_PATCH_COMPARISON: Op = Op {
    id: "getProviderPatchComparison",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/comparison",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/sbom`
///
/// Tag: Providers — Management. Success: `200`.
pub const GET_PROVIDER_SBOM: Op = Op {
    id: "getProviderSbom",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/{version}/sbom",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_PROVIDER_SHASUMS: Op = Op {
    id: "getProviderShasums",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS.sig`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_PROVIDER_SHASUMS_SIG: Op = Op {
    id: "getProviderShasumsSig",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/files/{upstream}/SHA256SUMS.sig",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/{upstream}/{filename}`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_PROVIDER_ZIP: Op = Op {
    id: "getProviderZip",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/files/{upstream}/{filename}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("filename", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_SELF_BUILT_SHASUMS: Op = Op {
    id: "getSelfBuiltShasums",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS.sig`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_SELF_BUILT_SHASUMS_SIG: Op = Op {
    id: "getSelfBuiltShasumsSig",
    method: Method::Get,
    template:
        "/v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/SHA256SUMS.sig",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/{filename}`
///
/// Tag: Registry — Providers. Success: `200`.
pub const GET_SELF_BUILT_ZIP: Op = Op {
    id: "getSelfBuiltZip",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/{version}/files/self_built/{lineage}/{filename}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
        ("lineage", ParamKind::Segment),
        ("filename", ParamKind::Segment),
    ],
    query: &["sig"],
    success: 200,
};
/// `GET /state/{org}/{key}`
///
/// Tag: State Backend. Success: `200`.
pub const GET_STATE: Op = Op {
    id: "getState",
    method: Method::Get,
    template: "/state/{org}/{key}",
    params: &[("org", ParamKind::Segment), ("key", ParamKind::RawPath)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/lock`
///
/// Tag: State Management. Success: `200`.
pub const GET_STATE_LOCK: Op = Op {
    id: "getStateLock",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/lock",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/versions/{versionId}/policy-results`
///
/// Tag: State Management. Success: `200`.
pub const GET_STATE_VERSION_POLICY_RESULTS: Op = Op {
    id: "getStateVersionPolicyResults",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/versions/{versionId}/policy-results",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/versions/{versionId}/resources`
///
/// Tag: State Management. Success: `200`.
pub const GET_STATE_VERSION_RESOURCES: Op = Op {
    id: "getStateVersionResources",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/versions/{versionId}/resources",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &["compare"],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/activity`
///
/// Tag: State Management. Success: `200`.
pub const GET_STATE_WORKSPACE_ACTIVITY: Op = Op {
    id: "getStateWorkspaceActivity",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/activity",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/decisions`
///
/// Tag: State Management. Success: `200`.
pub const GET_STATE_WORKSPACE_DECISIONS: Op = Op {
    id: "getStateWorkspaceDecisions",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/decisions",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &["limit"],
    success: 200,
};
/// `GET /.well-known/terraform.json`
///
/// Tag: Discovery. Success: `200`.
pub const GET_TERRAFORM_DISCOVERY: Op = Op {
    id: "getTerraformDiscovery",
    method: Method::Get,
    template: "/.well-known/terraform.json",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/policy-config`
///
/// Tag: Plans. Success: `200`.
pub const GET_WORKSPACE_PLAN_POLICY_CONFIG: Op = Op {
    id: "getWorkspacePlanPolicyConfig",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/policy-config",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /plans/{org}/{key}.tfplan`
///
/// Tag: Plans. Success: `200`.
pub const INGEST_PLAN: Op = Op {
    id: "ingestPlan",
    method: Method::Put,
    template: "/plans/{org}/{key}.tfplan",
    params: &[("org", ParamKind::Segment), ("key", ParamKind::RawPath)],
    query: &[],
    success: 200,
};
/// `POST /api/demo/join`
///
/// Tag: Public. Success: `200`.
pub const JOIN_DEMO_ORG: Op = Op {
    id: "joinDemoOrg",
    method: Method::Post,
    template: "/api/demo/join",
    params: &[],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/leave`
///
/// Tag: Orgs & Members. Success: `200`.
pub const LEAVE_ORG: Op = Op {
    id: "leaveOrg",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/leave",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/tokens`
///
/// Tag: Tokens. Success: `200`.
pub const LIST_ALL_TOKENS: Op = Op {
    id: "listAllTokens",
    method: Method::Get,
    template: "/api/tokens",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/billing/audit`
///
/// Tag: Billing. Success: `200`.
pub const LIST_AUDIT_EVENTS: Op = Op {
    id: "listAuditEvents",
    method: Method::Get,
    template: "/api/orgs/{orgId}/billing/audit",
    params: &[("orgId", ParamKind::Segment)],
    query: &["limit", "offset", "action", "before"],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/instance`
///
/// Tag: Policies. Success: `200`.
pub const LIST_INSTANCE_POLICIES: Op = Op {
    id: "listInstancePolicies",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/instance",
    params: &[("orgId", ParamKind::Segment)],
    query: &["artefact_type", "instance_key"],
    success: 200,
};
/// `GET /v1/modules/{namespace}/{name}/{provider}/versions`
///
/// Tag: Registry — Modules. Success: `200`.
pub const LIST_MODULE_VERSIONS: Op = Op {
    id: "listModuleVersions",
    method: Method::Get,
    template: "/v1/modules/{namespace}/{name}/{provider}/versions",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/namespaces/{namespace}/modules/{name}/versions`
///
/// Tag: Registry — Modules. Success: `200`.
pub const LIST_NAMESPACE_MODULE_VERSIONS: Op = Op {
    id: "listNamespaceModuleVersions",
    method: Method::Get,
    template: "/api/namespaces/{namespace}/modules/{name}/versions",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/namespaces/{namespace}/modules`
///
/// Tag: Registry — Modules. Success: `200`.
pub const LIST_NAMESPACE_MODULES: Op = Op {
    id: "listNamespaceModules",
    method: Method::Get,
    template: "/api/namespaces/{namespace}/modules",
    params: &[("namespace", ParamKind::Segment)],
    query: &["search", "limit", "offset"],
    success: 200,
};
/// `GET /api/orgs/{orgId}/members`
///
/// Tag: Orgs & Members. Success: `200`.
pub const LIST_ORG_MEMBERS: Op = Op {
    id: "listOrgMembers",
    method: Method::Get,
    template: "/api/orgs/{orgId}/members",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{orgSlug}/{namespace}/{type}/versions`
///
/// Tag: Registry — Providers. Success: `200`.
pub const LIST_ORG_PROVIDER_VERSIONS: Op = Op {
    id: "listOrgProviderVersions",
    method: Method::Get,
    template: "/v1/providers/{orgSlug}/{namespace}/{type}/versions",
    params: &[
        ("orgSlug", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/providers`
///
/// Tag: Providers — Management. Success: `200`.
pub const LIST_ORG_PROVIDERS: Op = Op {
    id: "listOrgProviders",
    method: Method::Get,
    template: "/api/orgs/{orgId}/providers",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/tokens`
///
/// Tag: Tokens. Success: `200`.
pub const LIST_ORG_TOKENS: Op = Op {
    id: "listOrgTokens",
    method: Method::Get,
    template: "/api/orgs/{orgId}/tokens",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs`
///
/// Tag: Orgs & Members. Success: `200`.
pub const LIST_ORGS: Op = Op {
    id: "listOrgs",
    method: Method::Get,
    template: "/api/orgs",
    params: &[],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/plans/{workspace}/versions`
///
/// Tag: Plans. Success: `200`.
pub const LIST_PLAN_VERSIONS: Op = Op {
    id: "listPlanVersions",
    method: Method::Get,
    template: "/api/v1/{orgId}/plans/{workspace}/versions",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &["limit", "offset"],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies`
///
/// Tag: Policies. Success: `200`.
pub const LIST_POLICIES: Op = Op {
    id: "listPolicies",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies",
    params: &[("orgId", ParamKind::Segment)],
    query: &["artefact_type"],
    success: 200,
};
/// `GET /api/orgs/{orgId}/policies/decisions`
///
/// Tag: Policies. Success: `200`.
pub const LIST_POLICY_DECISIONS: Op = Op {
    id: "listPolicyDecisions",
    method: Method::Get,
    template: "/api/orgs/{orgId}/policies/decisions",
    params: &[("orgId", ParamKind::Segment)],
    query: &[
        "interaction",
        "verdict",
        "policy_id",
        "since",
        "q",
        "req_id",
        "limit",
        "offset",
    ],
    success: 200,
};
/// `GET /api/orgs/{orgId}/provider-flows`
///
/// Tag: Provider Builds. Success: `200`.
pub const LIST_PROVIDER_FLOWS: Op = Op {
    id: "listProviderFlows",
    method: Method::Get,
    template: "/api/orgs/{orgId}/provider-flows",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /v1/providers/{namespace}/{type}/versions`
///
/// Tag: Registry — Providers. Success: `200`.
#[deprecated(
    note = "the spec marks `listProviderVersions` deprecated; see its description for the replacement"
)]
pub const LIST_PROVIDER_VERSIONS: Op = Op {
    id: "listProviderVersions",
    method: Method::Get,
    template: "/v1/providers/{namespace}/{type}/versions",
    params: &[
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/findings`
///
/// Tag: State Management. Success: `200`.
pub const LIST_STATE_FINDINGS: Op = Op {
    id: "listStateFindings",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/findings",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/versions`
///
/// Tag: State Management. Success: `200`.
pub const LIST_STATE_VERSIONS: Op = Op {
    id: "listStateVersions",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/versions",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &["limit", "offset"],
    success: 200,
};
/// `GET /api/v1/{orgId}/state`
///
/// Tag: State Management. Success: `200`.
pub const LIST_STATE_WORKSPACES: Op = Op {
    id: "listStateWorkspaces",
    method: Method::Get,
    template: "/api/v1/{orgId}/state",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/orgs/{orgId}/vcs-trust`
///
/// Tag: VCS Trust. Success: `200`.
pub const LIST_VCS_TRUST: Op = Op {
    id: "listVcsTrust",
    method: Method::Get,
    template: "/api/orgs/{orgId}/vcs-trust",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `GET /api/v1/{orgId}/state/{workspace}/providers`
///
/// Tag: State Management. Success: `200`.
pub const LIST_WORKSPACE_PROVIDERS: Op = Op {
    id: "listWorkspaceProviders",
    method: Method::Get,
    template: "/api/v1/{orgId}/state/{workspace}/providers",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/builds/{jobId}/override`
///
/// Tag: Provider Builds. Success: `200`.
pub const OVERRIDE_BUILD_GATE: Op = Op {
    id: "overrideBuildGate",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers/builds/{jobId}/override",
    params: &[("orgId", ParamKind::Segment), ("jobId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /v1/modules/{namespace}/{name}/{provider}/{version}/override-consumable`
///
/// Tag: Registry — Modules. Success: `200`.
pub const OVERRIDE_MODULE_VERSION_CONSUMABLE: Op = Op {
    id: "overrideModuleVersionConsumable",
    method: Method::Post,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/override-consumable",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/v1/{orgId}/state/{workspace}/versions/{versionId}/promote`
///
/// Tag: State Management. Success: `200`.
pub const PROMOTE_STATE_VERSION: Op = Op {
    id: "promoteStateVersion",
    method: Method::Post,
    template: "/api/v1/{orgId}/state/{workspace}/versions/{versionId}/promote",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
        ("versionId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /v1/modules/{namespace}/{name}/{provider}/{version}`
///
/// Tag: Registry — Modules. Success: `201`.
pub const PUBLISH_MODULE_VERSION: Op = Op {
    id: "publishModuleVersion",
    method: Method::Put,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &["description", "wait", "timeout"],
    success: 201,
};
/// `POST /state/{org}/{key}`
///
/// Tag: State Backend. Success: `200`.
pub const PUSH_STATE: Op = Op {
    id: "pushState",
    method: Method::Post,
    template: "/state/{org}/{key}",
    params: &[("org", ParamKind::Segment), ("key", ParamKind::RawPath)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/refresh-stats`
///
/// Tag: Providers — Management. Success: `200`.
pub const REFRESH_PROVIDER_VCS_STATS: Op = Op {
    id: "refreshProviderVcsStats",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/refresh-stats",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/auth/me`
///
/// Tag: Orgs & Members. Success: `200`.
pub const REGISTER_CURRENT_USER: Op = Op {
    id: "registerCurrentUser",
    method: Method::Post,
    template: "/api/auth/me",
    params: &[],
    query: &[],
    success: 200,
};
/// `POST /api/interest/{feature}`
///
/// Tag: Public. Success: `200`.
pub const REGISTER_FEATURE_INTEREST: Op = Op {
    id: "registerFeatureInterest",
    method: Method::Post,
    template: "/api/interest/{feature}",
    params: &[("feature", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/members/{userId}`
///
/// Tag: Orgs & Members. Success: `200`.
pub const REMOVE_ORG_MEMBER: Op = Op {
    id: "removeOrgMember",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/members/{userId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("userId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /v1/modules/{namespace}/{name}/{provider}/rescan-all`
///
/// Tag: Registry — Modules. Success: `200`.
pub const RESCAN_ALL_MODULE_VERSIONS: Op = Op {
    id: "rescanAllModuleVersions",
    method: Method::Post,
    template: "/v1/modules/{namespace}/{name}/{provider}/rescan-all",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /v1/modules/{namespace}/{name}/{provider}/{version}/rescan`
///
/// Tag: Registry — Modules. Success: `200`.
pub const RESCAN_MODULE_VERSION: Op = Op {
    id: "rescanModuleVersion",
    method: Method::Post,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/rescan",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/rescan`
///
/// Tag: Providers — Management. Success: `200`.
pub const RESCAN_ORG_PROVIDER: Op = Op {
    id: "rescanOrgProvider",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/rescan",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/v1/{orgId}/state/{workspace}/rescan`
///
/// Tag: State Management. Success: `200`.
pub const RESCAN_STATE_WORKSPACE: Op = Op {
    id: "rescanStateWorkspace",
    method: Method::Post,
    template: "/api/v1/{orgId}/state/{workspace}/rescan",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/v1/{orgId}/state/{workspace}/providers/rescan`
///
/// Tag: State Management. Success: `200`.
pub const RESCAN_WORKSPACE_PROVIDERS: Op = Op {
    id: "rescanWorkspaceProviders",
    method: Method::Post,
    template: "/api/v1/{orgId}/state/{workspace}/providers/rescan",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /api/orgs/{orgId}/tokens/{tokenId}`
///
/// Tag: Tokens. Success: `200`.
pub const REVOKE_ORG_TOKEN: Op = Op {
    id: "revokeOrgToken",
    method: Method::Delete,
    template: "/api/orgs/{orgId}/tokens/{tokenId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("tokenId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/tokens/{tokenId}/rotate`
///
/// Tag: Tokens. Success: `200`.
pub const ROTATE_ORG_TOKEN: Op = Op {
    id: "rotateOrgToken",
    method: Method::Post,
    template: "/api/orgs/{orgId}/tokens/{tokenId}/rotate",
    params: &[
        ("orgId", ParamKind::Segment),
        ("tokenId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `GET /v1/modules/search`
///
/// Tag: Registry — Modules. Success: `200`.
pub const SEARCH_MODULES: Op = Op {
    id: "searchModules",
    method: Method::Get,
    template: "/v1/modules/search",
    params: &[],
    query: &["q", "namespace", "limit", "offset"],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/billing/admin/{userId}`
///
/// Tag: Billing. Success: `200`.
pub const SET_BILLING_ADMIN: Op = Op {
    id: "setBillingAdmin",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/billing/admin/{userId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("userId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /v1/modules/{namespace}/{name}/{provider}/serve-override`
///
/// Tag: Registry — Modules. Success: `200`.
pub const SET_MODULE_SERVE_OVERRIDE: Op = Op {
    id: "setModuleServeOverride",
    method: Method::Put,
    template: "/v1/modules/{namespace}/{name}/{provider}/serve-override",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /v1/modules/{namespace}/{name}/{provider}/{version}/serve-override`
///
/// Tag: Registry — Modules. Success: `200`.
pub const SET_MODULE_VERSION_SERVE_OVERRIDE: Op = Op {
    id: "setModuleVersionServeOverride",
    method: Method::Put,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/serve-override",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /api/v1/{orgId}/plans/policy-config`
///
/// Tag: Plans. Success: `200`.
pub const SET_PLAN_POLICY_CONFIG: Op = Op {
    id: "setPlanPolicyConfig",
    method: Method::Put,
    template: "/api/v1/{orgId}/plans/policy-config",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PUT /api/orgs/{orgId}/policies/{id}/override`
///
/// Tag: Policies. Success: `200`.
pub const SET_POLICY_INSTANCE_OVERRIDE: Op = Op {
    id: "setPolicyInstanceOverride",
    method: Method::Put,
    template: "/api/orgs/{orgId}/policies/{id}/override",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PUT /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/override`
///
/// Tag: Providers — Management. Success: `200`.
pub const SET_PROVIDER_VERSION_SERVE_OVERRIDE: Op = Op {
    id: "setProviderVersionServeOverride",
    method: Method::Put,
    template:
        "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/override",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /api/v1/{orgId}/state/{workspace}/serve-override`
///
/// Tag: State Management. Success: `200`.
pub const SET_STATE_WORKSPACE_SERVE_OVERRIDE: Op = Op {
    id: "setStateWorkspaceServeOverride",
    method: Method::Put,
    template: "/api/v1/{orgId}/state/{workspace}/serve-override",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /api/v1/{orgId}/plans/{workspace}/policy-config`
///
/// Tag: Plans. Success: `200`.
pub const SET_WORKSPACE_PLAN_POLICY_CONFIG: Op = Op {
    id: "setWorkspacePlanPolicyConfig",
    method: Method::Put,
    template: "/api/v1/{orgId}/plans/{workspace}/policy-config",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `POST /api/forms`
///
/// Tag: Public. Success: `201`.
pub const SUBMIT_APP_FORM: Op = Op {
    id: "submitAppForm",
    method: Method::Post,
    template: "/api/forms",
    params: &[],
    query: &[],
    success: 201,
};
/// `POST /forms`
///
/// Tag: Public. Success: `201`.
pub const SUBMIT_PUBLIC_FORM: Op = Op {
    id: "submitPublicForm",
    method: Method::Post,
    template: "/forms",
    params: &[],
    query: &[],
    success: 201,
};
/// `POST /api/orgs/{orgId}/policies/{id}/toggle`
///
/// Tag: Policies. Success: `200`.
pub const TOGGLE_POLICY_ENABLED: Op = Op {
    id: "togglePolicyEnabled",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies/{id}/toggle",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/policies/{id}/toggle-enforcing`
///
/// Tag: Policies. Success: `200`.
pub const TOGGLE_POLICY_ENFORCING: Op = Op {
    id: "togglePolicyEnforcing",
    method: Method::Post,
    template: "/api/orgs/{orgId}/policies/{id}/toggle-enforcing",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `DELETE /v1/modules/{namespace}/{name}/{provider}/{version}/block`
///
/// Tag: Registry — Modules. Success: `200`.
pub const UNBLOCK_MODULE_VERSION: Op = Op {
    id: "unblockModuleVersion",
    method: Method::Delete,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/block",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /v1/modules/{namespace}/{name}/{provider}/deprecate`
///
/// Tag: Registry — Modules. Success: `200`.
pub const UNDEPRECATE_MODULE: Op = Op {
    id: "undeprecateModule",
    method: Method::Delete,
    template: "/v1/modules/{namespace}/{name}/{provider}/deprecate",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `DELETE /v1/modules/{namespace}/{name}/{provider}/{version}/deprecate`
///
/// Tag: Registry — Modules. Success: `200`.
pub const UNDEPRECATE_MODULE_VERSION: Op = Op {
    id: "undeprecateModuleVersion",
    method: Method::Delete,
    template: "/v1/modules/{namespace}/{name}/{provider}/{version}/deprecate",
    params: &[
        ("namespace", ParamKind::Segment),
        ("name", ParamKind::Segment),
        ("provider", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/policies/consumption-block`
///
/// Tag: Policies. Success: `200`.
pub const UPDATE_CONSUMPTION_BLOCK: Op = Op {
    id: "updateConsumptionBlock",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/policies/consumption-block",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/members/{userId}`
///
/// Tag: Orgs & Members. Success: `200`.
pub const UPDATE_ORG_MEMBER_ROLE: Op = Op {
    id: "updateOrgMemberRole",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/members/{userId}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("userId", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/notifications`
///
/// Tag: Orgs & Members. Success: `200`.
pub const UPDATE_ORG_NOTIFICATIONS: Op = Op {
    id: "updateOrgNotifications",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/notifications",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}`
///
/// Tag: Providers — Management. Success: `200`.
pub const UPDATE_ORG_PROVIDER: Op = Op {
    id: "updateOrgProvider",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/policies/{id}`
///
/// Tag: Policies. Success: `200`.
pub const UPDATE_POLICY: Op = Op {
    id: "updatePolicy",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/policies/{id}",
    params: &[("orgId", ParamKind::Segment), ("id", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PATCH /api/orgs/{orgId}/policies/settings`
///
/// Tag: Policies. Success: `200`.
pub const UPDATE_POLICY_SETTINGS: Op = Op {
    id: "updatePolicySettings",
    method: Method::Patch,
    template: "/api/orgs/{orgId}/policies/settings",
    params: &[("orgId", ParamKind::Segment)],
    query: &[],
    success: 200,
};
/// `PATCH /api/v1/{orgId}/state/{workspace}/settings`
///
/// Tag: State Management. Success: `200`.
pub const UPDATE_STATE_WORKSPACE_SETTINGS: Op = Op {
    id: "updateStateWorkspaceSettings",
    method: Method::Patch,
    template: "/api/v1/{orgId}/state/{workspace}/settings",
    params: &[
        ("orgId", ParamKind::Segment),
        ("workspace", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// `PUT /state/{org}/{key}`
///
/// Tag: State Backend. Success: `200`.
pub const UPLOAD_LOCK_FILE: Op = Op {
    id: "uploadLockFile",
    method: Method::Put,
    template: "/state/{org}/{key}",
    params: &[("org", ParamKind::Segment), ("key", ParamKind::RawPath)],
    query: &[],
    success: 200,
};
/// `POST /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/yank`
///
/// Tag: Provider Builds. Success: `200`.
pub const YANK_PROVIDER_VERSION: Op = Op {
    id: "yankProviderVersion",
    method: Method::Post,
    template: "/api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}/versions/{version}/yank",
    params: &[
        ("orgId", ParamKind::Segment),
        ("upstream", ParamKind::Segment),
        ("namespace", ParamKind::Segment),
        ("type", ParamKind::Segment),
        ("version", ParamKind::Segment),
    ],
    query: &[],
    success: 200,
};
/// Every operation generated here, for the contract test to walk.
pub const ALL: &[&Op] = &[
    #[allow(deprecated)]
    &ADD_ORG_MEMBER,
    #[allow(deprecated)]
    &ADD_ORG_PROVIDER,
    #[allow(deprecated)]
    &APPLY_POLICY_BASELINE,
    #[allow(deprecated)]
    &APPROVE_PLAN_VERSION,
    #[allow(deprecated)]
    &ATTEST_PROVIDER_POLICY_BLOCK,
    #[allow(deprecated)]
    &ATTEST_STATE_POLICY_BLOCK,
    #[allow(deprecated)]
    &AUTO_PATCH_PROVIDER_VERSION,
    #[allow(deprecated)]
    &BLOCK_MODULE_VERSION,
    #[allow(deprecated)]
    &BLOCKLIST_PROVIDER_VERSION,
    #[allow(deprecated)]
    &BUILD_PROVIDER_VERSION,
    #[allow(deprecated)]
    &CLAIM_ORG_DOMAIN,
    #[allow(deprecated)]
    &CREATE_BILLING_PORTAL_SESSION,
    #[allow(deprecated)]
    &CREATE_CHECKOUT_SESSION,
    #[allow(deprecated)]
    &CREATE_ORG,
    #[allow(deprecated)]
    &CREATE_ORG_TOKEN,
    #[allow(deprecated)]
    &CREATE_POLICY,
    #[allow(deprecated)]
    &CREATE_VCS_TRUST,
    #[allow(deprecated)]
    &DELETE_NAMESPACE_MODULE,
    #[allow(deprecated)]
    &DELETE_ORG,
    #[allow(deprecated)]
    &DELETE_ORG_PROVIDER,
    #[allow(deprecated)]
    &DELETE_POLICY,
    #[allow(deprecated)]
    &DELETE_STATE_RESOURCES,
    #[allow(deprecated)]
    &DELETE_STATE_WORKSPACE,
    #[allow(deprecated)]
    &DELETE_VCS_TRUST,
    #[allow(deprecated)]
    &DEPRECATE_MODULE,
    #[allow(deprecated)]
    &DEPRECATE_MODULE_VERSION,
    #[allow(deprecated)]
    &DOWNLOAD_MODULE_VERSION,
    #[allow(deprecated)]
    &FORCE_INSTANCE_POLICIES,
    #[allow(deprecated)]
    &FORCE_POLICY_INHERITANCE,
    #[allow(deprecated)]
    &FORCE_UNBLOCK_MODULE_VERSION,
    #[allow(deprecated)]
    &FORCE_UNLOCK_STATE,
    #[allow(deprecated)]
    &GET_ARTEFACT_POLICY_INPUT_SCHEMA,
    #[allow(deprecated)]
    &GET_ASYNC_API_JSON,
    #[allow(deprecated)]
    &GET_ASYNC_API_YAML,
    #[allow(deprecated)]
    &GET_BILLING,
    #[allow(deprecated)]
    &GET_BILLING_LIMITS,
    #[allow(deprecated)]
    &GET_CLI_DISCOVERY,
    #[allow(deprecated)]
    &GET_CONSUMPTION_BLOCK,
    #[allow(deprecated)]
    &GET_CURRENT_USER,
    #[allow(deprecated)]
    &GET_DOCS,
    #[allow(deprecated)]
    &GET_DOCS_OAUTH_REDIRECT,
    #[allow(deprecated)]
    &GET_ECHO_STATUS,
    #[allow(deprecated)]
    &GET_HEALTH,
    #[allow(deprecated)]
    &GET_MODULE_ARCHIVE,
    #[allow(deprecated)]
    &GET_MODULE_VERSION,
    #[allow(deprecated)]
    &GET_MODULE_VERSION_GRAPH,
    #[allow(deprecated)]
    &GET_MODULE_VERSION_STATUS,
    #[allow(deprecated)]
    &GET_OPEN_API_JSON,
    #[allow(deprecated)]
    &GET_OPEN_API_YAML,
    #[allow(deprecated)]
    &GET_ORG_ANALYTICS,
    #[allow(deprecated)]
    &GET_ORG_DEPENDENCY_GRAPH,
    #[allow(deprecated)]
    &GET_ORG_PROVIDER,
    #[allow(deprecated)]
    &GET_ORG_PROVIDER_DOWNLOAD,
    #[allow(deprecated)]
    &GET_ORG_PROVIDER_SHASUMS,
    #[allow(deprecated)]
    &GET_ORG_PROVIDER_SHASUMS_SIG,
    #[allow(deprecated)]
    &GET_ORG_PROVIDER_ZIP,
    #[allow(deprecated)]
    &GET_ORG_SELF_BUILT_SHASUMS,
    #[allow(deprecated)]
    &GET_ORG_SELF_BUILT_SHASUMS_SIG,
    #[allow(deprecated)]
    &GET_ORG_SELF_BUILT_ZIP,
    #[allow(deprecated)]
    &GET_ORG_SETTINGS,
    #[allow(deprecated)]
    &GET_PLAN_ATTESTATION_BY_HASH,
    #[allow(deprecated)]
    &GET_PLAN_ATTESTATION_BY_VERSION,
    #[allow(deprecated)]
    &GET_PLAN_POLICY_CONFIG,
    #[allow(deprecated)]
    &GET_PLAN_REPLACEMENTS_BY_PROVIDER,
    #[allow(deprecated)]
    &GET_PLAN_VERSION,
    #[allow(deprecated)]
    &GET_PLAN_VERSION_POLICY_RESULTS,
    #[allow(deprecated)]
    &GET_POLICY,
    #[allow(deprecated)]
    &GET_POLICY_COVERAGE,
    #[allow(deprecated)]
    &GET_POLICY_INPUT_SCHEMA,
    #[allow(deprecated)]
    &GET_POLICY_SETTINGS,
    #[allow(deprecated)]
    &GET_PORTAL_DISCOVERY,
    #[allow(deprecated)]
    &GET_PROVIDER_BUILD_FLOW,
    #[allow(deprecated)]
    &GET_PROVIDER_CONSUME_INSTRUCTIONS,
    #[allow(deprecated)]
    &GET_PROVIDER_DOWNLOAD,
    #[allow(deprecated)]
    &GET_PROVIDER_PATCH_COMPARISON,
    #[allow(deprecated)]
    &GET_PROVIDER_SBOM,
    #[allow(deprecated)]
    &GET_PROVIDER_SHASUMS,
    #[allow(deprecated)]
    &GET_PROVIDER_SHASUMS_SIG,
    #[allow(deprecated)]
    &GET_PROVIDER_ZIP,
    #[allow(deprecated)]
    &GET_SELF_BUILT_SHASUMS,
    #[allow(deprecated)]
    &GET_SELF_BUILT_SHASUMS_SIG,
    #[allow(deprecated)]
    &GET_SELF_BUILT_ZIP,
    #[allow(deprecated)]
    &GET_STATE,
    #[allow(deprecated)]
    &GET_STATE_LOCK,
    #[allow(deprecated)]
    &GET_STATE_VERSION_POLICY_RESULTS,
    #[allow(deprecated)]
    &GET_STATE_VERSION_RESOURCES,
    #[allow(deprecated)]
    &GET_STATE_WORKSPACE_ACTIVITY,
    #[allow(deprecated)]
    &GET_STATE_WORKSPACE_DECISIONS,
    #[allow(deprecated)]
    &GET_TERRAFORM_DISCOVERY,
    #[allow(deprecated)]
    &GET_WORKSPACE_PLAN_POLICY_CONFIG,
    #[allow(deprecated)]
    &INGEST_PLAN,
    #[allow(deprecated)]
    &JOIN_DEMO_ORG,
    #[allow(deprecated)]
    &LEAVE_ORG,
    #[allow(deprecated)]
    &LIST_ALL_TOKENS,
    #[allow(deprecated)]
    &LIST_AUDIT_EVENTS,
    #[allow(deprecated)]
    &LIST_INSTANCE_POLICIES,
    #[allow(deprecated)]
    &LIST_MODULE_VERSIONS,
    #[allow(deprecated)]
    &LIST_NAMESPACE_MODULE_VERSIONS,
    #[allow(deprecated)]
    &LIST_NAMESPACE_MODULES,
    #[allow(deprecated)]
    &LIST_ORG_MEMBERS,
    #[allow(deprecated)]
    &LIST_ORG_PROVIDER_VERSIONS,
    #[allow(deprecated)]
    &LIST_ORG_PROVIDERS,
    #[allow(deprecated)]
    &LIST_ORG_TOKENS,
    #[allow(deprecated)]
    &LIST_ORGS,
    #[allow(deprecated)]
    &LIST_PLAN_VERSIONS,
    #[allow(deprecated)]
    &LIST_POLICIES,
    #[allow(deprecated)]
    &LIST_POLICY_DECISIONS,
    #[allow(deprecated)]
    &LIST_PROVIDER_FLOWS,
    #[allow(deprecated)]
    &LIST_PROVIDER_VERSIONS,
    #[allow(deprecated)]
    &LIST_STATE_FINDINGS,
    #[allow(deprecated)]
    &LIST_STATE_VERSIONS,
    #[allow(deprecated)]
    &LIST_STATE_WORKSPACES,
    #[allow(deprecated)]
    &LIST_VCS_TRUST,
    #[allow(deprecated)]
    &LIST_WORKSPACE_PROVIDERS,
    #[allow(deprecated)]
    &OVERRIDE_BUILD_GATE,
    #[allow(deprecated)]
    &OVERRIDE_MODULE_VERSION_CONSUMABLE,
    #[allow(deprecated)]
    &PROMOTE_STATE_VERSION,
    #[allow(deprecated)]
    &PUBLISH_MODULE_VERSION,
    #[allow(deprecated)]
    &PUSH_STATE,
    #[allow(deprecated)]
    &REFRESH_PROVIDER_VCS_STATS,
    #[allow(deprecated)]
    &REGISTER_CURRENT_USER,
    #[allow(deprecated)]
    &REGISTER_FEATURE_INTEREST,
    #[allow(deprecated)]
    &REMOVE_ORG_MEMBER,
    #[allow(deprecated)]
    &RESCAN_ALL_MODULE_VERSIONS,
    #[allow(deprecated)]
    &RESCAN_MODULE_VERSION,
    #[allow(deprecated)]
    &RESCAN_ORG_PROVIDER,
    #[allow(deprecated)]
    &RESCAN_STATE_WORKSPACE,
    #[allow(deprecated)]
    &RESCAN_WORKSPACE_PROVIDERS,
    #[allow(deprecated)]
    &REVOKE_ORG_TOKEN,
    #[allow(deprecated)]
    &ROTATE_ORG_TOKEN,
    #[allow(deprecated)]
    &SEARCH_MODULES,
    #[allow(deprecated)]
    &SET_BILLING_ADMIN,
    #[allow(deprecated)]
    &SET_MODULE_SERVE_OVERRIDE,
    #[allow(deprecated)]
    &SET_MODULE_VERSION_SERVE_OVERRIDE,
    #[allow(deprecated)]
    &SET_PLAN_POLICY_CONFIG,
    #[allow(deprecated)]
    &SET_POLICY_INSTANCE_OVERRIDE,
    #[allow(deprecated)]
    &SET_PROVIDER_VERSION_SERVE_OVERRIDE,
    #[allow(deprecated)]
    &SET_STATE_WORKSPACE_SERVE_OVERRIDE,
    #[allow(deprecated)]
    &SET_WORKSPACE_PLAN_POLICY_CONFIG,
    #[allow(deprecated)]
    &SUBMIT_APP_FORM,
    #[allow(deprecated)]
    &SUBMIT_PUBLIC_FORM,
    #[allow(deprecated)]
    &TOGGLE_POLICY_ENABLED,
    #[allow(deprecated)]
    &TOGGLE_POLICY_ENFORCING,
    #[allow(deprecated)]
    &UNBLOCK_MODULE_VERSION,
    #[allow(deprecated)]
    &UNDEPRECATE_MODULE,
    #[allow(deprecated)]
    &UNDEPRECATE_MODULE_VERSION,
    #[allow(deprecated)]
    &UPDATE_CONSUMPTION_BLOCK,
    #[allow(deprecated)]
    &UPDATE_ORG_MEMBER_ROLE,
    #[allow(deprecated)]
    &UPDATE_ORG_NOTIFICATIONS,
    #[allow(deprecated)]
    &UPDATE_ORG_PROVIDER,
    #[allow(deprecated)]
    &UPDATE_POLICY,
    #[allow(deprecated)]
    &UPDATE_POLICY_SETTINGS,
    #[allow(deprecated)]
    &UPDATE_STATE_WORKSPACE_SETTINGS,
    #[allow(deprecated)]
    &UPLOAD_LOCK_FILE,
    #[allow(deprecated)]
    &YANK_PROVIDER_VERSION,
];
