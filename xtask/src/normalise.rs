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
pub const REQUIRED_EXCEPTIONS: &[(&str, &str)] = &[
    ("OrgMembership", "slug"),
    ("OrgMembership", "role"),
    ("StateVersionSummary", "id"),
    ("StateVersionSummary", "serial"),
    ("ProviderVersionRow", "version"),
    ("ProviderArtefactSummary", "namespace"),
    ("ProviderArtefactSummary", "type"),
    ("WorkspaceProvider", "namespace"),
    ("WorkspaceProvider", "type"),
    ("WorkspaceProvider", "current_version"),
];

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

/// Normalise `components.schemas` into typify-ready `(name, schema)` pairs.
pub fn normalise_schemas(schemas: &Map<String, Value>) -> (Vec<(String, Value)>, Vec<String>) {
    let mut collapsed = Vec::new();
    let out = schemas
        .iter()
        .map(|(name, schema)| {
            let mut s = schema.clone();
            rewrite_refs(&mut s);
            relax_required(&mut s, name);
            drop_external_formats(&mut s);
            if collapse_nullable_enum(&mut s) {
                collapsed.push(name.clone());
            }
            (name.clone(), s)
        })
        .collect();
    (out, collapsed)
}
