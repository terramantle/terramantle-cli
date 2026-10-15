//! Turn the OpenAPI 3.1 document's `components.schemas` into something typify
//! will accept, and apply this client's deserialisation policy while we are in
//! there.
//!
//! This is a RE-ROOTING, not a 3.1 -> 3.0 down-conversion. OpenAPI 3.1 schemas
//! are JSON Schema, and the spec happens to use none of the 2020-12 keywords
//! typify struggles with (`prefixItems`, `unevaluated*`, `dependent*`,
//! `$dynamicRef`, `if`/`then`/`else`, `propertyNames`) -- `golden` asserts that
//! rather than trusting it. What is left is moving `#/components/schemas/X` to
//! where typify looks for it, plus the `required` policy below.

use serde_json::{Map, Value};

/// `(schema, field)` pairs that stay REQUIRED despite the response policy.
///
/// The policy below makes every response field `Option<T>`, which is right for
/// a CLI that must keep working against an older deployment. But a handful of
/// keys are load-bearing identity that today's hand-written models already treat
/// as infallible, and making those `Option` would push an `unwrap` into every
/// call site for no safety gained. Keep them required, deliberately and visibly.
/// Names are the SPEC's schema names, not the CLI's old type names, and
/// [`check_exceptions`] fails codegen if one does not exist. The first draft of
/// this list said `OrgMembership` -- the hand-written Rust name -- which is not
/// a schema, so the exception silently did nothing and `slug` became `Option`.
/// A quietly ineffective entry is the exact failure mode this project exists to
/// remove, so it is checked rather than trusted.
pub const REQUIRED_EXCEPTIONS: &[(&str, &str)] = &[
    ("OrgSummary", "slug"),
    ("OrgSummary", "role"),
    ("OrgSummary", "name"),
    ("StateVersionSummary", "id"),
    ("StateVersionSummary", "serial"),
    ("ProviderVersionRow", "version"),
    ("ProviderArtefactSummary", "namespace"),
    ("ProviderArtefactSummary", "type"),
    ("WorkspaceProvider", "namespace"),
    ("WorkspaceProvider", "type"),
    ("WorkspaceProvider", "current_version"),
];

/// The `required` field names a schema declares, including through `allOf`.
fn required_names(def: &Value) -> Vec<String> {
    fn push_from(v: &Value, out: &mut Vec<String>) {
        if let Some(Value::Array(r)) = v.get("required") {
            out.extend(r.iter().filter_map(Value::as_str).map(str::to_string));
        }
    }
    let mut out = Vec::new();
    push_from(def, &mut out);
    if let Some(Value::Array(branches)) = def.get("allOf") {
        for b in branches {
            push_from(b, &mut out);
        }
    }
    out
}

/// Every [`REQUIRED_EXCEPTIONS`] entry must name a schema that exists and a
/// field that schema actually declares REQUIRED. Returns the bad entries.
pub fn check_exceptions(schemas: &Map<String, Value>) -> Vec<String> {
    let mut bad = Vec::new();
    for (schema, field) in REQUIRED_EXCEPTIONS {
        let Some(def) = schemas.get(*schema) else {
            bad.push(format!("{schema}.{field}: no such schema in the spec"));
            continue;
        };
        if !required_names(def).iter().any(|r| r == field) {
            bad.push(format!(
                "{schema}.{field}: the spec does not mark it required, so the exception is a no-op"
            ));
        }
    }
    bad
}

/// Rewrite every `$ref` in `v` from the OpenAPI pointer to typify's.
fn rewrite_refs(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get_mut("$ref") {
                if let Some(name) = r.strip_prefix("#/components/schemas/") {
                    *r = format!("#/definitions/{name}");
                }
            }
            for (_, child) in map.iter_mut() {
                rewrite_refs(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}

/// Strip `required` throughout `v`, keeping only the allowlisted keys for
/// `schema_name`.
///
/// `required` is ADVISORY for responses. The server declaring a field required
/// is a statement about the server, not a promise this client should crash over:
/// an older deployment that omits one should degrade a rendered row, not fail the
/// command. Strictness is kept where this client is the SENDER -- request bodies
/// are generated separately, with `required` intact.
fn relax_required(v: &mut Value, schema_name: &str) {
    match v {
        Value::Object(map) => {
            if let Some(Value::Array(req)) = map.get("required") {
                let kept: Vec<Value> = req
                    .iter()
                    .filter(|f| {
                        f.as_str().is_some_and(|f| {
                            REQUIRED_EXCEPTIONS.contains(&(schema_name, f))
                        })
                    })
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    map.remove("required");
                } else {
                    map.insert("required".into(), Value::Array(kept));
                }
            }
            for (_, child) in map.iter_mut() {
                relax_required(child, schema_name);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| relax_required(i, schema_name)),
        _ => {}
    }
}

/// Keywords that are legal in 2020-12 but that typify handles poorly. The spec
/// uses none of them today; if one arrives from the monorepo we want a loud
/// failure here rather than quietly wrong Rust.
pub const UNSUPPORTED_KEYWORDS: &[&str] = &[
    "prefixItems",
    "unevaluatedProperties",
    "unevaluatedItems",
    "dependentSchemas",
    "dependentRequired",
    "$anchor",
    "$dynamicRef",
    "$dynamicAnchor",
    "$recursiveRef",
    "propertyNames",
    "if",
    "then",
    "else",
];

/// Collect any [`UNSUPPORTED_KEYWORDS`] present, as `schema.path.keyword`.
pub fn find_unsupported(schemas: &Map<String, Value>) -> Vec<String> {
    fn walk(v: &Value, path: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for kw in UNSUPPORTED_KEYWORDS {
                    if map.contains_key(*kw) {
                        out.push(format!("{path}.{kw}"));
                    }
                }
                for (k, child) in map {
                    walk(child, &format!("{path}.{k}"), out);
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, &format!("{path}[{i}]"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (name, schema) in schemas {
        walk(schema, name, &mut out);
    }
    out
}

/// String formats typify maps onto external crates, which this crate does not
/// carry.
///
/// `uuid` becomes `::uuid::Uuid` and `date`/`date-time` become `chrono` types.
/// Three `date-time` fields and two `date` fields are not worth a `chrono`
/// dependency in a CLI that writes its own percent-encoder to avoid pulling in
/// `url`, and the ids these describe are passed straight through as strings.
/// Dropping the format yields `String`; the schema's `description` still says
/// what the value is, and typify carries that into the doc comment.
///
/// Note the spec's own timestamps are NOT affected: `Timestamp` is
/// `integer`/`int64` (epoch milliseconds), which maps to `i64` natively.
const DROPPED_STRING_FORMATS: &[&str] = &["uuid", "date", "date-time"];

/// Remove [`DROPPED_STRING_FORMATS`] wherever they appear. Returns how many.
fn drop_external_formats(v: &mut Value) -> usize {
    let mut n = 0;
    match v {
        Value::Object(map) => {
            let drop = map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|f| DROPPED_STRING_FORMATS.contains(&f));
            if drop {
                map.remove("format");
                n += 1;
            }
            for (_, child) in map.iter_mut() {
                n += drop_external_formats(child);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                n += drop_external_formats(item);
            }
        }
        _ => {}
    }
    n
}

/// Collapse a nullable enum to a plain one: drop `"null"` from `type` and
/// `null` from `enum`.
///
/// typify mishandles a NAMED nullable enum. Given `ServeOverride` as
/// `{type: ["string","null"], enum: ["block","allow",null]}` it emits BOTH
///
///     pub struct ServeOverride(pub Option<ServeOverride>);   // refers to itself
///     pub enum ServeOverride { Block, Allow }                // same name again
///
/// which is `error[E0428]: the name ServeOverride is defined multiple times`,
/// and the newtype is infinitely recursive besides. Collapsing to a plain enum
/// loses nothing: `required` is relaxed for responses, so every use site is
/// already `Option<ServeOverride>`, and `None` carries exactly the `null` the
/// schema describes (for `ServeOverride`, "clear the override").
///
/// Returns true if `v` was a nullable enum.
fn collapse_nullable_enum(v: &mut Value) -> bool {
    let Value::Object(map) = v else { return false };
    let is_nullable_type = map
        .get("type")
        .and_then(Value::as_array)
        .is_some_and(|t| t.iter().any(|x| x.as_str() == Some("null")));
    let has_null_in_enum = map
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|e| e.iter().any(Value::is_null));
    if !(is_nullable_type && has_null_in_enum) {
        return false;
    }

    let kept_types: Vec<Value> = map["type"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x.as_str() != Some("null"))
        .cloned()
        .collect();
    // A single remaining type is spelled as a bare string, which is what the
    // non-nullable form looks like and what typify reads most reliably.
    map.insert(
        "type".into(),
        if kept_types.len() == 1 {
            kept_types.into_iter().next().unwrap()
        } else {
            Value::Array(kept_types)
        },
    );
    let kept_variants: Vec<Value> = map["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| !x.is_null())
        .cloned()
        .collect();
    map.insert("enum".into(), Value::Array(kept_variants));
    true
}

/// Schemas that are arms of a real (non-nullable) `oneOf`/`anyOf` anywhere in
/// the document.
///
/// These are the one place `required` must be KEPT on a response. typify renders
/// such a union as `#[serde(untagged)]`, and serde picks the first arm that
/// deserializes. Since nothing denies unknown fields, an arm whose every field is
/// optional matches ANY object — so relaxing `required` here would make the first
/// arm swallow the second and the union would never report the other case.
///
/// Concretely: `ModulePublishPending` requires `ok`, `version_id`, `storage_key`
/// and `status`. Strip those and a settled publish deserializes as a pending one
/// with every field `None`, which is worse than not modelling the union at all.
pub fn union_arm_schemas(spec: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(Value::Array(arms)) = map.get("oneOf").or_else(|| map.get("anyOf")) {
                    let nullable = arms.iter().any(|a| a.get("type").and_then(Value::as_str) == Some("null"));
                    if arms.len() >= 2 && !nullable {
                        for arm in arms {
                            if let Some(r) = arm.get("$ref").and_then(Value::as_str) {
                                if let Some(n) = r.strip_prefix("#/components/schemas/") {
                                    out.push(n.to_string());
                                }
                            }
                        }
                    }
                }
                for child in map.values() {
                    walk(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(spec, &mut out);
    out.sort();
    out.dedup();
    out
}

/// Normalise `components.schemas` into typify-ready `(name, schema)` pairs.
///
/// `union_arms` keeps its `required` intact; see [`union_arm_schemas`].
pub fn normalise_schemas(
    schemas: &Map<String, Value>,
    union_arms: &[String],
) -> (Vec<(String, Value)>, Vec<String>) {
    let mut collapsed = Vec::new();
    let out = schemas
        .iter()
        .map(|(name, schema)| {
            let mut s = schema.clone();
            rewrite_refs(&mut s);
            if !union_arms.iter().any(|a| a == name) {
                relax_required(&mut s, name);
            }
            drop_external_formats(&mut s);
            if collapse_nullable_enum(&mut s) {
                collapsed.push(name.clone());
            }
            (name.clone(), s)
        })
        .collect();
    (out, collapsed)
}

/// Collect inline success-response schemas as `(TypeName, schema)` pairs.
///
/// Most operations answer with a `$ref` to a named component, which the models
/// already cover. The rest answer with an INLINE object -- the `{ orgs }`,
/// `{ providers }`, `{ workspaces }`, `{ versions }`, `{ lock }` envelopes, and
/// the module-publish union. Those shapes were previously hand-written in
/// `models.rs` and checked against nothing, which is the other half of the
/// problem this work exists to fix: paths AND payloads should come from the spec.
///
/// Named `<OperationId>Response` in PascalCase, so `listOrgs` yields
/// `ListOrgsResponse`.
pub fn inline_response_schemas(
    spec: &Value,
    operation_ids: &[String],
) -> Vec<(String, Value)> {
    const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head"];
    let mut out = Vec::new();
    let Some(paths) = spec["paths"].as_object() else {
        return out;
    };

    for item in paths.values() {
        let Some(item) = item.as_object() else { continue };
        for method in METHODS {
            let Some(op) = item.get(*method) else { continue };
            let Some(id) = op["operationId"].as_str() else { continue };
            if !operation_ids.iter().any(|w| w == id) {
                continue;
            }
            // The success response the registry records: lowest 2xx.
            let Some(responses) = op["responses"].as_object() else { continue };
            let mut codes: Vec<&String> = responses
                .keys()
                .filter(|c| c.starts_with('2'))
                .collect();
            codes.sort();
            let Some(code) = codes.first() else { continue };
            let schema = &responses[*code]["content"]["application/json"]["schema"];
            if schema.is_null() || schema.get("$ref").is_some() {
                continue; // a named component already covers it
            }

            let mut s = schema.clone();
            rewrite_refs(&mut s);
            let type_name = format!("{}Response", pascal(id));
            relax_required(&mut s, &type_name);
            drop_external_formats(&mut s);
            out.push((type_name, s));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `listOrgs` -> `ListOrgs`.
fn pascal(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len());
    let mut upper_next = true;
    for ch in camel.chars() {
        if ch == '_' || ch == '-' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}
