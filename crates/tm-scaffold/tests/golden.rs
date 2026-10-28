//! Golden-file tests for every rendered pipeline.
//!
//! The scaffolder had no golden files, only inline `assert!(content.contains(…))`
//! in the renderer's own module. That is how five separate defects shipped at
//! once — a `contains` assertion tells you a fragment is present, never that the
//! file as a whole is the file you meant to write. The committed goldens here
//! make every change to generated CI show up as a reviewable diff.
//!
//! Regenerate after an intentional change:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test -p tm-scaffold --test golden
//! ```
//!
//! Then **read the diff**. These files are what lands in users' repositories.

use std::path::{Path, PathBuf};

use tm_scaffold::{
    desired_files, Artefact, CiAuth, CiConfig, Discovery, Manifest, Structure, Terramate,
    VcsProvider,
};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The variants worth pinning. Not the full matrix: these are the combinations
/// a real repo lands on, and each exercises a different code path through the
/// conditional blocks.
fn variants() -> Vec<(&'static str, Manifest)> {
    let m = |artefact: Artefact, vcs: VcsProvider, auth: CiAuth, lint: bool, docs: bool| Manifest {
        structure: Structure::Mono,
        artefact,
        org: "acme-demo".into(),
        api_url: None,
        vcs,
        discovery: Some(Discovery::default_for(artefact)),
        terramate: match artefact {
            Artefact::Workspaces => Some(Terramate::default()),
            Artefact::Modules => None,
        },
        ci: CiConfig {
            auth,
            lint,
            docs,
            ..CiConfig::default()
        },
    };
    vec![
        (
            "github-workspaces",
            m(
                Artefact::Workspaces,
                VcsProvider::Github,
                CiAuth::Oidc,
                true,
                true,
            ),
        ),
        (
            "gitlab-workspaces",
            m(
                Artefact::Workspaces,
                VcsProvider::Gitlab,
                CiAuth::Oidc,
                true,
                true,
            ),
        ),
        (
            "github-modules",
            m(
                Artefact::Modules,
                VcsProvider::Github,
                CiAuth::Oidc,
                true,
                true,
            ),
        ),
        (
            "gitlab-modules",
            m(
                Artefact::Modules,
                VcsProvider::Gitlab,
                CiAuth::Oidc,
                true,
                true,
            ),
        ),
        // Bot swaps the whole credential path, so it is pinned separately.
        (
            "github-workspaces-bot",
            m(
                Artefact::Workspaces,
                VcsProvider::Github,
                CiAuth::Bot,
                true,
                true,
            ),
        ),
        (
            "gitlab-workspaces-bot",
            m(
                Artefact::Workspaces,
                VcsProvider::Gitlab,
                CiAuth::Bot,
                true,
                true,
            ),
        ),
        // Everything optional switched off: proves a disabled feature leaves no
        // trace rather than a commented-out stub.
        (
            "github-workspaces-minimal",
            m(
                Artefact::Workspaces,
                VcsProvider::Github,
                CiAuth::Oidc,
                false,
                false,
            ),
        ),
    ]
}

/// The pipeline file for a manifest, whichever provider it targets.
fn pipeline(m: &Manifest) -> String {
    desired_files(m)
        .into_iter()
        .find(|f| {
            f.path == Path::new(".github/workflows/terramantle.yml")
                || f.path == Path::new(".gitlab-ci.yml")
        })
        .expect("a pipeline is always rendered")
        .content
}

fn check(name: &str, actual: &str) -> Result<(), String> {
    // A name that already carries an extension is used verbatim, so the
    // Terramate config is not stored as `terramate.tm.hcl.yml`.
    let file = if name.contains('.') {
        name.to_string()
    } else {
        format!("{name}.yml")
    };
    let path = golden_dir().join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return Ok(());
    }
    let expected = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "missing golden {}\n  regenerate: UPDATE_GOLDEN=1 cargo test -p tm-scaffold --test golden",
            path.display()
        )
    })?;
    if expected == actual {
        return Ok(());
    }
    // Point at the first differing line: a whole-file dump of a 400-line
    // workflow buries the one thing that changed.
    let (mut line, mut detail) = (0usize, String::from("<trailing content differs>"));
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            line = i + 1;
            detail = format!("expected: {e}\n  actual:   {a}");
            break;
        }
    }
    Err(format!(
        "{name} differs from its golden at line {line}\n  {detail}\n  \
         regenerate (and READ the diff): UPDATE_GOLDEN=1 cargo test -p tm-scaffold --test golden"
    ))
}

#[test]
fn rendered_pipelines_match_their_goldens() {
    let mut failures = Vec::new();
    for (name, manifest) in variants() {
        if let Err(e) = check(name, &pipeline(&manifest)) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "\n\n{}\n", failures.join("\n\n"));
}

/// The Terramate root config is generated too, and its `workspace` expression
/// is the one value that silently breaks `state publish` when it is wrong.
#[test]
fn terramate_config_matches_its_golden() {
    let (_, m) = variants()
        .into_iter()
        .find(|(n, _)| *n == "github-workspaces")
        .unwrap();
    let tm = desired_files(&m)
        .into_iter()
        .find(|f| f.path == Path::new("terramate.tm.hcl"))
        .expect("terramate config is rendered");
    if let Err(e) = check("terramate.tm.hcl", &tm.content) {
        panic!("\n\n{e}\n");
    }
}

/// Rendering must not depend on the environment or on iteration order: a
/// golden that only matches sometimes is worse than none.
#[test]
fn rendering_is_stable_across_runs() {
    for (name, m) in variants() {
        assert_eq!(pipeline(&m), pipeline(&m), "{name} is not deterministic");
    }
}
