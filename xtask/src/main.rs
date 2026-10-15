//! Development tooling. Not a workspace member, so `cargo build --all --release`
//! and cargo-dist never see it and the shipped crates keep `tempfile` as their
//! only dev-dependency.
//!
//! ```text
//! cargo xtask codegen           regenerate crates/tm-api/src/generated/
//! cargo xtask codegen --check   fail if the committed output is stale
//! cargo xtask spec pull [--from URL|PATH]
//! ```
//!
//! `codegen` reads ONLY the vendored `spec/openapi.json`, never the network, so
//! `cargo test` is offline and deterministic. `spec pull` is the single place
//! that fetches.

mod normalise;
mod operations;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("codegen") => codegen(args.iter().any(|a| a == "--check")),
        Some("spec") if args.get(1).map(String::as_str) == Some("pull") => {
            let from = args
                .iter()
                .position(|a| a == "--from")
                .and_then(|i| args.get(i + 1))
                .cloned();
            spec_pull(from)
        }
        _ => {
            eprintln!(
                "usage:\n  \
                 cargo xtask codegen [--check]\n  \
                 cargo xtask spec pull [--from URL|PATH]"
            );
            std::process::exit(2);
        }
    }
}

/// Default source for `spec pull`: the deployed contract.
const DEFAULT_SPEC_URL: &str = "https://registry.terramantle.dev/openapi.json";

fn spec_pull(from: Option<String>) -> Res<()> {
    let root = repo_root();
    let src = from.unwrap_or_else(|| DEFAULT_SPEC_URL.to_string());
    let body = if src.starts_with("http://") || src.starts_with("https://") {
        let out = Command::new("curl").args(["-sSf", &src]).output()?;
        if !out.status.success() {
            return Err(format!("curl {src} failed: {}", String::from_utf8_lossy(&out.stderr)).into());
        }
        String::from_utf8(out.stdout)?
    } else {
        std::fs::read_to_string(&src)?
    };

    // Parse before writing: a truncated or HTML response must not land in the
    // repo as if it were a spec.
    let spec: Value = serde_json::from_str(&body)?;
    let version = spec["info"]["version"].as_str().unwrap_or("unknown").to_string();
    let paths = spec["paths"].as_object().map_or(0, |p| p.len());
    if paths == 0 {
        return Err("fetched document has no paths — refusing to vendor it".into());
    }

    std::fs::write(root.join("spec/openapi.json"), &body)?;
    let digest = sha256_hex(&root.join("spec/openapi.json"))?;
    std::fs::write(root.join("spec/openapi.sha256"), format!("{digest}\n"))?;

    // Record provenance that means something on another machine. An absolute
    // path on whoever ran this is useless; the monorepo commit the spec was
    // built from is the thing you would actually want to look up.
    let (source, origin) = describe_source(&src);
    std::fs::write(
        root.join("spec/SPEC_VERSION.toml"),
        format!(
            "# Written by `cargo xtask spec pull`. The spec this build was generated against.\n\
             # `cargo xtask codegen` reads spec/openapi.json and NEVER the network, so a build\n\
             # is reproducible from this pin alone.\n\
             version = {version:?}\n\
             sha256 = {digest:?}\n\
             source = {source:?}\n\
             origin = {origin:?}\n\
             paths = {paths}\n"
        ),
    )?;
    println!("vendored spec {version} ({paths} paths)\n  sha256 {digest}\n  from   {src}");
    println!("now run: cargo xtask codegen");
    Ok(())
}

/// Turn a `--from` into `(source, origin)` for the pin file.
///
/// A URL is already portable. A local path is not, so it is reduced to the
/// repository-relative path plus that repository's HEAD — which is what someone
/// reading the pin later actually needs.
fn describe_source(src: &str) -> (String, String) {
    if src.starts_with("http://") || src.starts_with("https://") {
        return (src.to_string(), "deployed".to_string());
    }
    let path = Path::new(src);
    let dir = path.parent().unwrap_or(Path::new("."));
    let head = Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());
    let rel = Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["ls-files", "--full-name"])
        .arg(path.file_name().unwrap_or_default())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match (rel, head) {
        (Some(rel), Some(head)) => (rel, format!("git:{head}")),
        _ => (
            path.file_name().unwrap_or_default().to_string_lossy().to_string(),
            "local".to_string(),
        ),
    }
}

fn sha256_hex(path: &Path) -> Res<String> {
    let out = Command::new("shasum").args(["-a", "256"]).arg(path).output()?;
    if !out.status.success() {
        return Err("shasum failed".into());
    }
    Ok(String::from_utf8(out.stdout)?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}

/// The two extension methods nothing can generate. Asserted by NAME so a third
/// arriving from the monorepo fails codegen instead of vanishing.
const EXPECTED_EXTENSION_METHODS: &[&str] = &["/state/{org}/{key}/lock x-lock", "/state/{org}/{key}/lock x-unlock"];

fn codegen(check: bool) -> Res<()> {
    let root = repo_root();
    let raw = std::fs::read_to_string(root.join("spec/openapi.json"))?;
    let spec: Value = serde_json::from_str(&raw)?;

    // ── guard: a 2020-12 keyword typify mishandles has arrived ───────────────
    let schemas = spec["components"]["schemas"].as_object().ok_or("no schemas")?;
    let unsupported = normalise::find_unsupported(schemas);
    if !unsupported.is_empty() {
        return Err(format!(
            "spec uses {} schema keyword(s) the generator does not handle:\n  {}",
            unsupported.len(),
            unsupported.join("\n  ")
        )
        .into());
    }

    // ── operations first: their inline response schemas feed the models ─────
    let (ops, tombstones, extensions) = operations::collect(&spec);
    let op_ids: Vec<String> = ops.iter().map(|o| o.id.clone()).collect();

    // ── models ───────────────────────────────────────────────────────────────
    // A `required` exception that names nothing is worse than no exception: it
    // looks deliberate and does nothing.
    let bad = normalise::check_exceptions(schemas);
    if !bad.is_empty() {
        return Err(format!(
            "REQUIRED_EXCEPTIONS has {} entry/entries that do not match the spec:\n  {}",
            bad.len(),
            bad.join("\n  ")
        )
        .into());
    }

    let union_arms = normalise::union_arm_schemas(&spec);
    let (mut normalised, collapsed) = normalise::normalise_schemas(schemas, &union_arms);
    let inline = normalise::inline_response_schemas(&spec, &op_ids);
    let inline_count = inline.len();
    // A component of the same name would be shadowed; there is none today, and
    // a collision should be a loud failure rather than a silent override.
    for (name, _) in &inline {
        if normalised.iter().any(|(n, _)| n == name) {
            return Err(format!(
                "generated response type `{name}` collides with a component schema of the \
                 same name; rename the component or the operation"
            )
            .into());
        }
    }
    normalised.extend(inline);
    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_struct_builder(false);
    let mut ts = typify::TypeSpace::new(&settings);
    let pairs: Vec<(String, schemars::schema::Schema)> = normalised
        .iter()
        .map(|(n, s)| {
            let sch = serde_json::from_value(s.clone())
                .map_err(|e| format!("schema {n}: {e}"))
                .expect("schema parses as JSON Schema");
            (n.clone(), sch)
        })
        .collect();
    ts.add_ref_types(pairs)?;

    let models_body = prettyplease::unparse(&syn::parse_file(&ts.to_stream().to_string())?);
    let models = format!(
        "// @generated by `cargo xtask codegen` from spec/openapi.json. DO NOT EDIT.\n\
         //\n\
         //! Request and response types, generated from the spec's `components.schemas`.\n\
         //!\n\
         //! Response fields are `Option<T>` and no struct denies unknown fields, which is\n\
         //! this crate's long-standing rubric: a deployment that omits a field the spec\n\
         //! calls required should degrade a rendered row, not fail the command. See\n\
         //! `xtask/src/normalise.rs` for the policy and the handful of exceptions.\n\
         #![allow(clippy::all)]\n\n\
         {models_body}"
    );

    // ── operations ───────────────────────────────────────────────────────────
    let mut found = extensions.clone();
    found.sort();
    let mut expected = EXPECTED_EXTENSION_METHODS.to_vec();
    expected.sort();
    if found != expected {
        return Err(format!(
            "non-standard method extensions changed.\n  expected: {expected:?}\n  found:    {found:?}\n\
             These cannot be generated (OpenAPI has no syntax for LOCK/UNLOCK). If a new one is \
             legitimate, add it to EXPECTED_EXTENSION_METHODS and hand-write its client."
        )
        .into());
    }

    let operations =
        operations::render(&ops, &tombstones) + &operations::render_manifest(&ops);
    let operations = prettyplease::unparse(&syn::parse_file(&operations)?);

    let r#mod = "// @generated by `cargo xtask codegen` from spec/openapi.json. DO NOT EDIT.\n\
                 //! Spec-derived types and the operation registry.\n\n\
                 pub mod models;\n\
                 pub mod operations;\n"
        .to_string();

    let formatted = rustfmt_module(&[
        ("models.rs", &models),
        ("operations.rs", &operations),
        ("mod.rs", &r#mod),
    ])?;

    let mut stale = Vec::new();
    for (name, content) in &formatted {
        let path = root.join("crates/tm-api/src/generated").join(name);
        if check {
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if &current != content {
                stale.push(name.clone());
            }
        } else {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, content)?;
        }
    }

    if check {
        if stale.is_empty() {
            println!("generated code is up to date ({} operations)", ops.len());
            return Ok(());
        }
        return Err(format!(
            "generated code is stale:\n  {}\nrun `cargo xtask codegen` and commit the result",
            stale.join("\n  ")
        )
        .into());
    }

    println!(
        "models:     {} component schemas + {inline_count} inline response type(s) -> crates/tm-api/src/generated/models.rs",
        schemas.len()
    );
    if !union_arms.is_empty() {
        println!("            union arms keeping `required`: {}", union_arms.join(", "));
    }
    if !collapsed.is_empty() {
        println!("            collapsed nullable enum(s): {}", collapsed.join(", "));
    }
    println!("operations: {} -> crates/tm-api/src/generated/operations.rs", ops.len());
    println!("tombstoned: {} (no constants emitted: {})", tombstones.len(), tombstones.join(", "));
    println!("extensions: {} skipped (LOCK/UNLOCK)", extensions.len());
    Ok(())
}

/// Format the generated module exactly as `cargo fmt` would.
///
/// Two subtleties, both learned the hard way:
///
/// 1. Formatting is done on FILES, not through stdin. The two disagree -- piped
///    rustfmt leaves the closing `*/` of a block doc comment one space short of
///    where `cargo fmt` puts it, so a stdin-formatted file is dirty the moment
///    CI runs `cargo fmt --all -- --check`. By path it matches, and is
///    idempotent.
/// 2. All the files are written to one temp directory FIRST, because `mod.rs`
///    declares `pub mod models;` and rustfmt resolves that relative to the file
///    it is given. Formatting `mod.rs` alone fails with "failed to resolve mod".
///    (`--skip-children` would avoid this but is not on stable.)
fn rustfmt_module(files: &[(&str, &String)]) -> Res<Vec<(String, String)>> {
    let dir = std::env::temp_dir().join(format!("tm-xtask-fmt-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;

    for (name, content) in files {
        std::fs::write(dir.join(name), content.as_str())?;
    }

    let mut cmd = Command::new("rustfmt");
    cmd.args(["--edition", "2021", "--quiet"]);
    for (name, _) in files {
        cmd.arg(dir.join(name));
    }
    let status = cmd.status()?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("rustfmt failed on generated output".into());
    }

    let mut out = Vec::new();
    for (name, _) in files {
        out.push((
            (*name).to_string(),
            std::fs::read_to_string(dir.join(name))?,
        ));
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}
