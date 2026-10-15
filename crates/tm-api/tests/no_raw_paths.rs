//! The ratchet: endpoint paths must come from the generated registry, not from
//! `format!` strings.
//!
//! Hand-written paths are how all six of the bugs that prompted this work got
//! in. Three were removed server-side and answered 410; one was deprecated; one
//! carried a trailing slash the worker 404s on; one named a route that never
//! existed. Nothing could see any of them, because a `format!` string is not
//! checked against anything.
//!
//! `KNOWN_RAW_PATHS` is the burn-down list, not a permanent exemption. The test
//! fails BOTH ways on purpose: a new raw path is a regression, and a removed one
//! means this list is stale and should shrink. Migrating a module therefore ends
//! with deleting its entry here.

/// Endpoint modules, with their source embedded at compile time.
const SOURCES: &[(&str, &str)] = &[
    ("providers.rs", include_str!("../src/providers.rs")),
    ("modules.rs", include_str!("../src/modules.rs")),
    ("state.rs", include_str!("../src/state.rs")),
    ("orgs.rs", include_str!("../src/orgs.rs")),
];

/// Path literals still written by hand, as `(file, count)`.
///
/// Every one of these is a path the spec already describes; the entry exists
/// only because its module has not been migrated to `tm_api::ops` yet.
/// Empty: every endpoint module now builds its paths from the spec.
///
/// `fully_migrated_modules_have_no_path_literals` holds all four at zero from
/// here, so none of them can quietly go back to `format!`.
const KNOWN_RAW_PATHS: &[(&str, usize)] = &[];

/// Count string literals that look like an API path, in PRODUCTION code.
///
/// Two deliberate exclusions. Comments are skipped, because a doc comment
/// citing the endpoint a method calls is useful documentation. And everything
/// from `#[cfg(test)]` onward is skipped, because a test that asserts
/// `ops::LIST_ORGS.path(&[]) == "/api/orgs"` is pinning the generated path —
/// the opposite of the problem, and exactly what a migrated module should have.
fn count_path_literals(src: &str) -> usize {
    const PREFIXES: &[&str] = &["\"/api", "\"/v1", "\"/state", "\"/plans", "\"/.well-known"];
    let production = src.split("#[cfg(test)]").next().unwrap_or(src);
    production
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(|l| PREFIXES.iter().filter(|p| l.contains(**p)).count())
        .sum()
}

#[test]
fn raw_path_literals_only_where_recorded() {
    let mut drift = Vec::new();
    for (file, src) in SOURCES {
        let found = count_path_literals(src);
        let expected = KNOWN_RAW_PATHS
            .iter()
            .find(|(f, _)| f == file)
            .map_or(0, |(_, n)| *n);
        if found != expected {
            drift.push(format!(
                "  {file}: {found} hand-written path literal(s), expected {expected}"
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "hand-written endpoint paths drifted from the burn-down list:\n{}\n\n\
         If you ADDED one: don't — use `tm_api::ops::<OPERATION_ID>` and `Op::path()`, so the \
         path comes from the spec and a removed endpoint fails the build.\n\
         If you REMOVED one: lower the count in KNOWN_RAW_PATHS (and delete the entry at 0).",
        drift.join("\n")
    );
}

/// The migrated modules must stay migrated: once a file is off the list it may
/// never regain a hand-written path.
#[test]
fn fully_migrated_modules_have_no_path_literals() {
    for (file, src) in SOURCES {
        let on_list = KNOWN_RAW_PATHS.iter().any(|(f, n)| f == file && *n > 0);
        if !on_list {
            assert_eq!(
                count_path_literals(src),
                0,
                "{file} was migrated to the generated registry; it must not build paths by hand again"
            );
        }
    }
}
