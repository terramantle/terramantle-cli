//! Emit the operation registry: one `Op` constant per OpenAPI operation.
//!
//! Keyed on `operationId`, deliberately. A registry that matched a URL against
//! path templates would happily match the CLI's dead
//! `/api/orgs/{org}/provider-foundry/providers-usage` against the TOMBSTONE
//! template `/api/orgs/{orgId}/provider-foundry/{path}` and report it fine.
//! `operationId` makes that failure mode impossible: a removed operation has no
//! constant, so referring to it does not compile.

use serde_json::Value;
use std::fmt::Write as _;

/// Tags whose operations never belong in a user-facing CLI: scanner ingest, the
/// Argo build fleet's result callbacks, policy-bundle serving, Stripe/R2 hooks.
const SKIPPED_TAG_PREFIXES: &[&str] = &["Internal —", "Internal -", "Webhooks"];

/// Paths whose final parameter is a router WILDCARD that may contain slashes.
///
/// `components.parameters.StateKey` says it outright: "The protocol router
/// matches a wildcard, so this may contain slashes (`prod/backend`) — which
/// OpenAPI path templating cannot express." Percent-encoding such a parameter
/// yields `/state/acme/prod%2F.terraform.lock.hcl`, a 404. These parameters are
/// therefore joined verbatim, and `lock push` -- the most-run CI command --
/// depends on it.
const WILDCARD_PARAMS: &[(&str, &str)] = &[
    ("/state/{org}/{key}", "key"),
    ("/state/{org}/{key}/lock", "key"),
    ("/plans/{org}/{key}.tfplan", "key"),
];

const HTTP_METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head"];

pub struct Operation {
    pub id: String,
    pub method: String,
    pub path: String,
    /// `(name, is_wildcard)` in template order.
    pub path_params: Vec<(String, bool)>,
    pub query_params: Vec<String>,
    pub success: u16,
    pub deprecated: bool,
    pub tag: String,
}

/// Resolve a `$ref` into `components`, or return the value unchanged.
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

/// The parameter names appearing in a path template, in order.
fn template_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        out.push(rest[open + 1..open + close].to_string());
        rest = &rest[open + close..];
    }
    out
}

fn screaming_snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, ch) in camel.chars().enumerate() {
        if ch.is_ascii_uppercase() && i != 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

/// Collect the operations worth generating.
///
/// Returns `(operations, skipped_tombstones, skipped_extension_methods)`.
pub fn collect(spec: &Value) -> (Vec<Operation>, Vec<String>, Vec<String>) {
    let mut ops = Vec::new();
    let mut tombstones = Vec::new();
    let mut extensions = Vec::new();

    let paths = spec["paths"].as_object().expect("paths");
    for (path, item) in paths {
        let item_obj = item.as_object().expect("path item");

        // `x-lock` / `x-unlock` carry Terraform's LOCK and UNLOCK, which are not
        // HTTP methods OpenAPI can express. Nothing can generate them; record
        // them so a THIRD such extension appearing is a loud failure, not a
        // silent omission.
        for key in item_obj.keys() {
            if key.starts_with("x-") {
                extensions.push(format!("{path} {key}"));
            }
        }

        for method in HTTP_METHODS {
            let Some(op) = item_obj.get(*method) else { continue };
            let id = op["operationId"].as_str().expect("operationId").to_string();
            let tag = op["tags"][0].as_str().unwrap_or("").to_string();
            if SKIPPED_TAG_PREFIXES.iter().any(|p| tag.starts_with(p)) {
                continue;
            }

            let responses = op["responses"].as_object().expect("responses");
            let codes: Vec<&String> = responses.keys().collect();

            // A tombstone: the endpoint is gone and 410 is all it can say. Emit
            // NOTHING, so a call site referring to it fails to compile.
            if codes.len() == 1 && codes[0] == "410" {
                tombstones.push(id);
                continue;
            }

            let success = codes
                .iter()
                .filter_map(|c| c.parse::<u16>().ok())
                .filter(|c| (200..300).contains(c))
                .min()
                .unwrap_or(200);

            // Merge path-item-level and operation-level parameters.
            let mut declared: Vec<&Value> = Vec::new();
            for src in [item_obj.get("parameters"), op.get("parameters")] {
                if let Some(Value::Array(list)) = src {
                    declared.extend(list.iter());
                }
            }
            let resolved: Vec<&Value> = declared.iter().map(|p| resolve(spec, p)).collect();

            let query_params: Vec<String> = resolved
                .iter()
                .filter(|p| p["in"].as_str() == Some("query"))
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect();

            // Path parameters come from the TEMPLATE, in template order -- the
            // order a caller passes arguments in. The declared list is only
            // consulted to confirm each one exists.
            let path_params: Vec<(String, bool)> = template_params(path)
                .into_iter()
                .map(|name| {
                    let wildcard = WILDCARD_PARAMS
                        .iter()
                        .any(|(p, n)| *p == path.as_str() && *n == name.as_str());
                    (name, wildcard)
                })
                .collect();

            ops.push(Operation {
                id,
                method: method.to_uppercase(),
                path: path.clone(),
                path_params,
                query_params,
                success,
                deprecated: op["deprecated"].as_bool().unwrap_or(false),
                tag,
            });
        }
    }
    ops.sort_by(|a, b| a.id.cmp(&b.id));
    tombstones.sort();
    extensions.sort();
    (ops, tombstones, extensions)
}

/// Render `operations.rs`.
pub fn render(ops: &[Operation], tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated by `cargo xtask codegen` from spec/openapi.json. DO NOT EDIT.\n\
         //\n\
         //! Every operation the Terramantle API serves, as a constant keyed on its\n\
         //! `operationId`.\n\
         //!\n\
         //! Endpoint paths are NOT written by hand anywhere in this crate. A path that\n\
         //! the spec does not describe has no constant here, so calling it does not\n\
         //! compile; a path the spec describes cannot acquire a typo or a trailing\n\
         //! slash, because the template below is copied from the spec verbatim.\n\n",
    );
    let _ = writeln!(
        out,
        "use crate::spec::{{Method, Op, ParamKind}};\n\n\
         /// Operations the spec marks as removed (their only response is `410`).\n\
         /// Listed for the contract test; deliberately NOT given constants, so a call\n\
         /// site that still refers to one fails to compile rather than 410 at runtime.\n\
         pub const TOMBSTONED_OPERATION_IDS: &[&str] = &[{}];\n",
        tombstones
            .iter()
            .map(|t| format!("\n    {t:?},"))
            .collect::<String>()
            + if tombstones.is_empty() { "" } else { "\n" }
    );

    for op in ops {
        let _ = writeln!(out, "/// `{} {}`", op.method, op.path);
        let _ = writeln!(out, "///");
        let _ = writeln!(out, "/// Tag: {}. Success: `{}`.", op.tag, op.success);
        if op.deprecated {
            let _ = writeln!(
                out,
                "#[deprecated(note = \"the spec marks `{}` deprecated; see its description for the replacement\")]",
                op.id
            );
        }
        let params = op
            .path_params
            .iter()
            .map(|(n, wild)| {
                format!(
                    "({n:?}, ParamKind::{})",
                    if *wild { "RawPath" } else { "Segment" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let queries = op
            .query_params
            .iter()
            .map(|q| format!("{q:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "pub const {}: Op = Op {{\n    \
                 id: {:?},\n    \
                 method: Method::{},\n    \
                 template: {:?},\n    \
                 params: &[{}],\n    \
                 query: &[{}],\n    \
                 success: {},\n\
             }};\n",
            screaming_snake(&op.id),
            op.id,
            match op.method.as_str() {
                "GET" => "Get",
                "PUT" => "Put",
                "POST" => "Post",
                "DELETE" => "Delete",
                "PATCH" => "Patch",
                "HEAD" => "Head",
                m => panic!("unexpected method {m}"),
            },
            op.path,
            params,
            queries,
            op.success,
        );
    }
    out
}

/// Every generated operation id, for the contract test.
pub fn render_manifest(ops: &[Operation]) -> String {
    let mut out = String::from(
        "\n/// Every operation generated here, for the contract test to walk.\npub const ALL: &[&Op] = &[",
    );
    for op in ops {
        let _ = write!(out, "\n    #[allow(deprecated)] &{},", screaming_snake(&op.id));
    }
    out.push_str("\n];\n");
    out
}
