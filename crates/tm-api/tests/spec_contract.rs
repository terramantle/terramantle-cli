//! The contract test: every generated operation must still match the vendored
//! spec, and the CLI's own call sites must still exist in it.
//!
//! This is the CLI-side half of a three-link chain. The monorepo's
//! `openapiContract.test.ts` binds the spec to Hono's real route table, so the
//! spec's paths and methods are machine-verified against the server.
//! `cargo xtask codegen --check` binds the generated tree to the vendored spec.
//! This file guards the join: that the generator did what the spec says, and
//! that nothing the CLI calls has quietly been removed.
//!
//! What it cannot catch is the spec being wrong about the deployment — response
//! SHAPES are not machine-checked server-side. That needs a credentialed smoke
//! run, which is deliberately not part of `cargo test`.

use serde_json::Value;
use tm_api::spec::{Method, ParamKind};

/// The vendored spec, embedded so the test is offline and deterministic.
const SPEC: &str = include_str!("../../../spec/openapi.json");

fn spec() -> Value {
    serde_json::from_str(SPEC).expect("vendored spec parses")
}

fn method_key(m: Method) -> &'static str {
    match m {
        Method::Get => "get",
        Method::Put => "put",
        Method::Post => "post",
        Method::Delete => "delete",
        Method::Patch => "patch",
        Method::Head => "head",
    }
}

/// Parameter names in a path template, in order.
fn template_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push(rest[open + 1..open + close].to_string());
        rest = &rest[open + close..];
    }
    out
}

#[test]
fn every_generated_operation_exists_in_the_spec() {
    let spec = spec();
    let paths = spec["paths"].as_object().expect("paths");

    for op in tm_api::ops::ALL {
        let item = paths
            .get(op.template)
            .unwrap_or_else(|| panic!("{}: template {} is not in the spec", op.id, op.template));
        let entry = item.get(method_key(op.method)).unwrap_or_else(|| {
            panic!(
                "{}: {} {} is not in the spec",
                op.id, op.method, op.template
            )
        });
        assert_eq!(
            entry["operationId"].as_str(),
            Some(op.id),
            "{}: operationId disagrees with the spec at {} {}",
            op.id,
            op.method,
            op.template
        );
    }
}

#[test]
fn path_parameters_match_the_template_in_name_and_order() {
    for op in tm_api::ops::ALL {
        let from_template = template_params(op.template);
        let declared: Vec<String> = op.params.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(
            declared, from_template,
            "{}: params must match the template in name AND order, since `path()` \
             substitutes positionally",
            op.id
        );
    }
}

/// `Op::path` substitutes positionally, so a template placeholder with no
/// matching parameter would be left in the URL verbatim.
#[test]
fn building_a_path_leaves_no_placeholder_behind() {
    for op in tm_api::ops::ALL {
        let args: Vec<&str> = op.params.iter().map(|_| "x").collect();
        let built = op.path(&args).unwrap_or_else(|e| panic!("{}: {e}", op.id));
        assert!(
            !built.contains('{') && !built.contains('}'),
            "{}: built path still has a placeholder: {built}",
            op.id
        );
        assert!(built.starts_with('/'), "{}: {built}", op.id);
    }
}

/// The trailing slash that broke `state ls`: `/api/v1/{org}/state/` 404s while
/// `/api/v1/{org}/state` is a real route. Templates come from the spec, so no
/// generated operation can carry one.
#[test]
fn no_template_has_a_trailing_slash() {
    for op in tm_api::ops::ALL {
        assert!(
            op.template == "/" || !op.template.ends_with('/'),
            "{}: template {} has a trailing slash; the worker 404s on those",
            op.id,
            op.template
        );
    }
}

/// Tombstones get no constant, so a call site referring to one does not
/// compile. This checks the list is still exactly the spec's removed set — if
/// the server tombstones another endpoint, codegen drops it and this test names
/// it.
#[test]
fn tombstoned_operations_are_exactly_the_specs_removed_set() {
    let spec = spec();
    let mut from_spec: Vec<String> = Vec::new();
    for (_, item) in spec["paths"].as_object().expect("paths") {
        for (key, op) in item.as_object().expect("path item") {
            if !["get", "put", "post", "delete", "patch", "head"].contains(&key.as_str()) {
                continue;
            }
            let codes: Vec<&String> = op["responses"]
                .as_object()
                .expect("responses")
                .keys()
                .collect();
            if codes.len() == 1 && codes[0] == "410" {
                from_spec.push(op["operationId"].as_str().expect("operationId").to_string());
            }
        }
    }
    from_spec.sort();
    let mut generated = tm_api::ops::TOMBSTONED_OPERATION_IDS.to_vec();
    generated.sort_unstable();
    assert_eq!(
        from_spec, generated,
        "the spec's removed endpoints and the recorded tombstones have diverged"
    );

    // And none of them leaked into the registry.
    for op in tm_api::ops::ALL {
        assert!(
            !tm_api::ops::TOMBSTONED_OPERATION_IDS.contains(&op.id),
            "{} is tombstoned but still has a constant",
            op.id
        );
    }
}

/// The three endpoints whose removal this whole exercise started from. If any
/// reappears as a live operation, that is a deliberate server change and this
/// test should be updated knowingly.
#[test]
fn the_removed_provider_foundry_routes_have_no_constants() {
    for dead in [
        "removedProviderFoundryRoute",
        "removedProviderMirrorRoute",
        "betaUpgrade",
    ] {
        assert!(
            tm_api::ops::TOMBSTONED_OPERATION_IDS.contains(&dead),
            "{dead} should still be tombstoned"
        );
        assert!(
            !tm_api::ops::ALL.iter().any(|op| op.id == dead),
            "{dead} must not have a constant"
        );
    }
}

/// `provider-foundry` and `provider-mirror` live on in the spec ONLY as 410
/// tombstones. No generated operation should route through either prefix.
#[test]
fn no_generated_operation_targets_a_removed_prefix() {
    for op in tm_api::ops::ALL {
        for dead in [
            "provider-foundry",
            "provider-mirror",
            "providers-usage",
            "providers-overview",
        ] {
            assert!(
                !op.template.contains(dead),
                "{} targets the removed prefix `{dead}`: {}",
                op.id,
                op.template
            );
        }
    }
}

/// Internal scanner / build-fleet / webhook surfaces must not be reachable from
/// a user-facing CLI binary.
#[test]
fn internal_and_webhook_surfaces_are_not_generated() {
    for op in tm_api::ops::ALL {
        for prefix in [
            "/internal/",
            "/api/internal/",
            "/webhooks/",
            "/billing/webhook",
        ] {
            assert!(
                !op.template.starts_with(prefix),
                "{} is an internal surface and should have been skipped: {}",
                op.id,
                op.template
            );
        }
    }
}

/// Only the protocol routers' wildcard keys may be joined unencoded. Marking
/// any other parameter `RawPath` would let a value containing `/` rewrite the
/// route.
#[test]
fn only_the_protocol_wildcards_are_raw() {
    let allowed = [
        ("uploadLockFile", "key"),
        ("getState", "key"),
        ("pushState", "key"),
        ("ingestPlan", "key"),
        ("lockState", "key"),
        ("unlockState", "key"),
    ];
    for op in tm_api::ops::ALL {
        for (name, kind) in op.params {
            if *kind == ParamKind::RawPath {
                assert!(
                    allowed.contains(&(op.id, name)),
                    "{}: parameter `{name}` is RawPath (joined unencoded) but is not a \
                     documented protocol wildcard",
                    op.id
                );
            }
        }
    }
}

/// Declared query parameters must match the spec's, so a caller cannot pass a
/// parameter the server ignores.
#[test]
fn query_parameters_match_the_spec() {
    let spec = spec();
    let paths = spec["paths"].as_object().expect("paths");

    // Resolve a `$ref` into the document.
    fn resolve<'a>(spec: &'a Value, v: &'a Value) -> &'a Value {
        v.get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/"))
            .map(|rest| {
                let mut cur = spec;
                for seg in rest.split('/') {
                    cur = &cur[seg];
                }
                cur
            })
            .unwrap_or(v)
    }

    for op in tm_api::ops::ALL {
        let item = &paths[op.template];
        let entry = &item[method_key(op.method)];
        let mut expected: Vec<String> = Vec::new();
        for src in [item.get("parameters"), entry.get("parameters")] {
            if let Some(Value::Array(list)) = src {
                for p in list {
                    let p = resolve(&spec, p);
                    if p["in"].as_str() == Some("query") {
                        if let Some(n) = p["name"].as_str() {
                            expected.push(n.to_string());
                        }
                    }
                }
            }
        }
        let mut got: Vec<String> = op.query.iter().map(|q| q.to_string()).collect();
        expected.sort();
        got.sort();
        assert_eq!(expected, got, "{}: query parameters diverge", op.id);
    }
}

/// The operations the CLI actually calls. Listed explicitly so that a server
/// removal shows up here by name rather than as a 410 in someone's terminal.
const CLI_OPERATIONS: &[&str] = &[
    "getCliDiscovery",
    "listOrgs",
    "listOrgProviders",
    "getOrgProvider",
    "listOrgProviderVersions",
    "searchModules",
    "listModuleVersions",
    "getModuleVersion",
    "publishModuleVersion",
    "listStateWorkspaces",
    "listStateVersions",
    "promoteStateVersion",
    "getStateLock",
    "forceUnlockState",
    "listWorkspaceProviders",
    "uploadLockFile",
];

#[test]
fn every_operation_the_cli_calls_is_live() {
    for want in CLI_OPERATIONS {
        let found = tm_api::ops::ALL.iter().find(|op| op.id == *want);
        assert!(
            found.is_some(),
            "the CLI calls `{want}`, which has no generated operation. Either the spec \
             removed it (check TOMBSTONED_OPERATION_IDS) or it was never there."
        );
    }
}

/// Nothing the CLI calls may be deprecated. This is belt-and-braces: a
/// deprecated constant already fails the build at the call site under
/// `-D warnings`, but that only fires once the call site exists, and this names
/// the problem during a migration.
#[test]
fn no_operation_the_cli_calls_is_deprecated() {
    let spec = spec();
    for want in CLI_OPERATIONS {
        let Some(op) = tm_api::ops::ALL.iter().find(|o| o.id == *want) else {
            continue;
        };
        let entry = &spec["paths"][op.template][method_key(op.method)];
        assert_ne!(
            entry["deprecated"].as_bool(),
            Some(true),
            "the CLI calls `{want}`, which the spec marks deprecated. Its description names \
             the replacement."
        );
    }
}
