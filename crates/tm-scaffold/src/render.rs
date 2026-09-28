//! Render the desired file set from a [`Manifest`] (SCAFFOLD-PUBLISH-AUTH.md §2).
//!
//! Every file `init` writes / `upgrade` manages is produced here from the manifest
//! alone, so rendering is a pure, deterministic function of the manifest — the
//! precondition for `upgrade` being a real diff. Two classes of file exist:
//!
//! * [`FileClass::Managed`] — deterministic from the manifest (CI pipeline,
//!   CODEOWNERS). `upgrade` re-renders and diffs these; each carries a provenance
//!   header so a user edit is detectable.
//! * [`FileClass::Once`] — a starting point the user owns and edits (skeleton
//!   `.tf`, backend stub, `.gitignore`, commit config). `init` creates them if
//!   absent; `upgrade` never rewrites or prunes them.

use std::path::PathBuf;

use crate::manifest::{Artefact, CiAuth, CiConfig, Manifest, Structure, VcsProvider};

/// How `upgrade` treats a generated file. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// Re-rendered and diffed on every `upgrade`.
    Managed,
    /// Created once by `init`; never rewritten or pruned.
    Once,
}

/// One file the scaffolder wants on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    /// Path relative to the repo root.
    pub path: PathBuf,
    pub content: String,
    /// Template id recorded in the lock + provenance header, e.g. `github/modules`.
    pub template: String,
    pub template_version: u32,
    pub class: FileClass,
}

/// Provenance header version; bump when a managed template's layout changes.
pub const TEMPLATE_VERSION: u32 = 1;

/// The substring every managed file's provenance header carries. `upgrade` scans
/// for this to recognise files it owns — the lock-free replacement for a manifest
/// lock (§3): a marked file no longer in the desired set is pruned.
pub const PROVENANCE_MARKER: &str = "managed by terramantle · template=";

/// The default registry URL the templates reference (mirrors tm-config).
const DEFAULT_REGISTRY: &str = "https://registry.terramantle.dev";

/// Every path a managed file could occupy across both VCS providers and artefact
/// types. `upgrade` checks these for prune candidates (a marked file here that the
/// current manifest no longer wants — e.g. the other VCS's pipeline after a switch).
pub fn candidate_managed_paths() -> Vec<PathBuf> {
    [
        ".github/workflows/terramantle.yml",
        ".gitlab-ci.yml",
        ".github/CODEOWNERS",
        ".gitlab/CODEOWNERS",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

/// Build the full desired file set for a manifest.
///
/// Excludes `terramantle.hcl` itself — that is user-owned config, written once by
/// `init` and never rewritten by `upgrade` (it is the *input*, not an output).
pub fn desired_files(m: &Manifest) -> Vec<GeneratedFile> {
    let mut files = Vec::new();
    files.push(ci_pipeline(m));
    files.push(codeowners(m));
    files.extend(skeleton(m));
    files
}

/// A `#`-commented provenance header. `Managed` files carry the
/// "do not edit above this line" contract; `Once` files get no header.
fn header(template: &str, class: FileClass) -> String {
    match class {
        FileClass::Managed => format!(
            "# {PROVENANCE_MARKER}{template} · v={TEMPLATE_VERSION} · do not edit above this line\n"
        ),
        FileClass::Once => String::new(),
    }
}

fn registry_url(m: &Manifest) -> &str {
    m.api_url.as_deref().unwrap_or(DEFAULT_REGISTRY)
}

// ---- CI pipeline ----------------------------------------------------------

fn ci_pipeline(m: &Manifest) -> GeneratedFile {
    let (path, content, template) = match (m.vcs, m.artefact) {
        (VcsProvider::Github, Artefact::Modules) => (
            ".github/workflows/terramantle.yml",
            github_modules(m),
            "github/modules",
        ),
        (VcsProvider::Github, Artefact::Workspaces) => (
            ".github/workflows/terramantle.yml",
            github_workspaces(m),
            "github/workspaces",
        ),
        (VcsProvider::Gitlab, Artefact::Modules) => {
            (".gitlab-ci.yml", gitlab_modules(m), "gitlab/modules")
        }
        (VcsProvider::Gitlab, Artefact::Workspaces) => {
            (".gitlab-ci.yml", gitlab_workspaces(m), "gitlab/workspaces")
        }
    };
    GeneratedFile {
        path: PathBuf::from(path),
        content,
        template: template.to_string(),
        template_version: TEMPLATE_VERSION,
        class: FileClass::Managed,
    }
}

fn tf_matrix(ci: &CiConfig) -> String {
    let quoted: Vec<String> = ci
        .terraform_versions
        .iter()
        .map(|v| format!("\"{v}\""))
        .collect();
    format!("[{}]", quoted.join(", "))
}

fn github_modules(m: &Manifest) -> String {
    let ci = &m.ci;
    let mut s = header("github/modules", FileClass::Managed);
    s.push_str("#\n# Emitted by `terramantle init` for artefact=modules · vcs=github.\n");
    s.push_str("# The pipeline uses the terramantle CLI itself for auth and publishing.\n#\n");
    s.push_str("# Required permissions: contents:write (push version tags); ");
    s.push_str(match ci.auth {
        CiAuth::Oidc => "id-token:write (mint the ambient OIDC token).\n",
        CiAuth::Bot => "set repo secrets TERRAMANTLE_BOT_CLIENT_ID/_SECRET (auth=bot).\n",
    });
    s.push_str("name: terramantle-modules\n\n");
    s.push_str("on:\n  push:\n    branches: [main]\n  pull_request:\n\n");
    s.push_str("permissions:\n  contents: write\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  id-token: write\n");
    }
    s.push('\n');

    s.push_str("jobs:\n");
    s.push_str("  validate:\n    runs-on: ubuntu-latest\n");
    s.push_str(&format!(
        "    strategy:\n      matrix:\n        tf: {}\n    steps:\n",
        tf_matrix(ci)
    ));
    s.push_str("      - uses: actions/checkout@v4\n        with: { fetch-depth: 0 }\n");
    s.push_str(
        "      - uses: hashicorp/setup-terraform@v3\n        with: { terraform_version: \"${{ matrix.tf }}\" }\n",
    );
    if ci.tofu {
        s.push_str("      - uses: opentofu/setup-opentofu@v1\n");
    }
    s.push_str("      - uses: terramantle/setup-cli@v1\n");
    s.push_str("      - run: terraform fmt -check -recursive\n");
    s.push_str("      - run: terramantle modules changed\n");
    if ci.lint {
        s.push_str("      - uses: terraform-linters/setup-tflint@v4\n");
        s.push_str("      - run: tflint --recursive\n");
    }
    s.push('\n');

    s.push_str("  publish:\n    needs: validate\n");
    s.push_str("    if: github.ref == 'refs/heads/main'\n    runs-on: ubuntu-latest\n    steps:\n");
    s.push_str("      - uses: actions/checkout@v4\n        with: { fetch-depth: 0 }\n");
    s.push_str("      - uses: hashicorp/setup-terraform@v3\n");
    s.push_str("      - uses: terramantle/setup-cli@v1\n");
    if ci.terraform_docs {
        s.push_str("      - uses: terraform-docs/gh-actions@v1\n");
        s.push_str(
            "        with: { find-dir: ., output-file: README.md, output-method: inject }\n",
        );
    }
    s.push_str("      - name: Publish changed modules\n        env:\n");
    s.push_str(&format!("          TERRAMANTLE_ORG: {}\n", m.org));
    if ci.auth == CiAuth::Bot {
        s.push_str(
            "          TERRAMANTLE_BOT_CLIENT_ID: ${{ secrets.TERRAMANTLE_BOT_CLIENT_ID }}\n",
        );
        s.push_str(
            "          TERRAMANTLE_BOT_CLIENT_SECRET: ${{ secrets.TERRAMANTLE_BOT_CLIENT_SECRET }}\n",
        );
    }
    s.push_str("        run: terramantle modules publish --ci\n");
    s
}

fn github_workspaces(m: &Manifest) -> String {
    let ci = &m.ci;
    let mut s = header("github/workspaces", FileClass::Managed);
    s.push_str(
        "#\n# Emitted by `terramantle init` for artefact=workspaces (states) · vcs=github.\n",
    );
    s.push_str(
        "# Publishing a workspace repo uploads each .terraform.lock.hcl to the registry.\n#\n",
    );
    s.push_str(match ci.auth {
        CiAuth::Oidc => {
            "# Required permissions: id-token:write (ambient OIDC → terramantle bearer).\n"
        }
        CiAuth::Bot => "# auth=bot: set repo secrets TERRAMANTLE_BOT_CLIENT_ID/_SECRET.\n",
    });
    s.push_str("name: terramantle-workspaces\n\n");
    s.push_str("on:\n  push:\n    branches: [main]\n  pull_request:\n\n");
    s.push_str("permissions:\n  contents: read\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  id-token: write\n");
    }
    s.push('\n');

    s.push_str("jobs:\n");
    s.push_str("  plan:\n    runs-on: ubuntu-latest\n    steps:\n");
    s.push_str("      - uses: actions/checkout@v4\n");
    s.push_str("      - uses: hashicorp/setup-terraform@v3\n");
    s.push_str("      - uses: terramantle/setup-cli@v1\n");
    s.push_str("      - run: terraform init -backend=false\n");
    s.push_str("      - run: terraform fmt -check -recursive\n");
    s.push_str("      - run: terraform validate\n");
    if ci.security_scan {
        s.push_str("      - run: terramantle lock push --dry-run --fail-on-atrisk\n");
    }
    s.push('\n');

    s.push_str("  publish:\n    needs: plan\n");
    s.push_str("    if: github.ref == 'refs/heads/main'\n    runs-on: ubuntu-latest\n    steps:\n");
    s.push_str("      - uses: actions/checkout@v4\n");
    s.push_str("      - uses: hashicorp/setup-terraform@v3\n");
    s.push_str("      - uses: terramantle/setup-cli@v1\n");
    s.push_str("      - name: Upload lock files for every workspace\n        env:\n");
    s.push_str(&format!("          TERRAMANTLE_ORG: {}\n", m.org));
    if ci.auth == CiAuth::Bot {
        s.push_str(
            "          TERRAMANTLE_BOT_CLIENT_ID: ${{ secrets.TERRAMANTLE_BOT_CLIENT_ID }}\n",
        );
        s.push_str(
            "          TERRAMANTLE_BOT_CLIENT_SECRET: ${{ secrets.TERRAMANTLE_BOT_CLIENT_SECRET }}\n",
        );
    }
    s.push_str("        run: terramantle state publish --ci --fail-on-atrisk\n");
    s
}

fn gitlab_oidc_block(m: &Manifest) -> String {
    format!(
        ".oidc: &oidc\n  id_tokens:\n    TERRAMANTLE_ID_TOKEN:\n      aud: {}\n\n",
        registry_url(m)
    )
}

fn gitlab_modules(m: &Manifest) -> String {
    let ci = &m.ci;
    let mut s = header("gitlab/modules", FileClass::Managed);
    s.push_str(
        "#\n# Emitted by `terramantle init` for artefact=modules · vcs=gitlab (.gitlab-ci.yml).\n",
    );
    s.push_str("# auth=oidc mints a CI JWT (aud=registry); auth=bot uses masked CI variables.\n");
    s.push_str("stages: [validate, publish]\n\n");
    s.push_str("default:\n  image: ghcr.io/terramantle/cli:1\n\n");
    s.push_str(&format!("variables:\n  TERRAMANTLE_ORG: {}\n\n", m.org));
    if ci.auth == CiAuth::Oidc {
        s.push_str(&gitlab_oidc_block(m));
    }

    s.push_str("validate:\n  stage: validate\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  <<: *oidc\n");
    }
    s.push_str("  script:\n");
    s.push_str("    - terraform fmt -check -recursive\n");
    s.push_str("    - terramantle modules changed\n");
    if ci.lint {
        s.push_str("    - tflint --recursive\n");
    }
    s.push('\n');

    s.push_str("publish:\n  stage: publish\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  <<: *oidc\n");
    }
    s.push_str("  rules:\n    - if: '$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH'\n  script:\n");
    if ci.terraform_docs {
        s.push_str(
            "    - terraform-docs markdown table --output-file README.md --output-mode inject .\n",
        );
    }
    s.push_str("    - terramantle modules publish --ci\n");
    s
}

fn gitlab_workspaces(m: &Manifest) -> String {
    let ci = &m.ci;
    let mut s = header("gitlab/workspaces", FileClass::Managed);
    s.push_str(
        "#\n# Emitted by `terramantle init` for artefact=workspaces (states) · vcs=gitlab.\n",
    );
    s.push_str("# Publishing uploads each workspace's .terraform.lock.hcl to the registry.\n");
    s.push_str("stages: [plan, publish]\n\n");
    s.push_str("default:\n  image: ghcr.io/terramantle/cli:1\n\n");
    s.push_str(&format!("variables:\n  TERRAMANTLE_ORG: {}\n\n", m.org));
    if ci.auth == CiAuth::Oidc {
        s.push_str(&gitlab_oidc_block(m));
    }

    s.push_str("plan:\n  stage: plan\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  <<: *oidc\n");
    }
    s.push_str("  script:\n");
    s.push_str("    - terraform init -backend=false\n");
    s.push_str("    - terraform fmt -check -recursive\n");
    s.push_str("    - terraform validate\n");
    if ci.security_scan {
        s.push_str("    - terramantle lock push --dry-run --fail-on-atrisk\n");
    }
    s.push('\n');

    s.push_str("publish:\n  stage: publish\n");
    if ci.auth == CiAuth::Oidc {
        s.push_str("  <<: *oidc\n");
    }
    s.push_str("  rules:\n    - if: '$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH'\n  script:\n");
    s.push_str("    - terramantle state publish --ci --fail-on-atrisk\n");
    s
}

// ---- .gitignore -----------------------------------------------------------

/// The ignore lines every terramantle repo needs.
pub const GITIGNORE_LINES: &[&str] = &[".terraform/", "*.tfstate", "*.tfstate.*"];

/// Idempotently ensure the terramantle ignore lines are present in a `.gitignore`.
///
/// Returns `Some(new_content)` when a change is needed (missing lines appended
/// under a `# terramantle` marker), or `None` when the file already covers them —
/// so `upgrade` only rewrites `.gitignore` when it actually must. Unlike managed
/// files this is a *merge*, never a clobber: the user's own entries are preserved.
pub fn merge_gitignore(existing: Option<&str>) -> Option<String> {
    let current = existing.unwrap_or("");
    let present: std::collections::BTreeSet<&str> = current.lines().map(str::trim).collect();
    let missing: Vec<&str> = GITIGNORE_LINES
        .iter()
        .copied()
        .filter(|l| !present.contains(*l))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = String::from(current);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str("# terramantle\n");
    for line in missing {
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

// ---- CODEOWNERS -----------------------------------------------------------

fn codeowners(m: &Manifest) -> GeneratedFile {
    let path = match m.vcs {
        VcsProvider::Github => ".github/CODEOWNERS",
        VcsProvider::Gitlab => ".gitlab/CODEOWNERS",
    };
    let mut s = header("codeowners", FileClass::Managed);
    s.push_str("# Ownership stub scaffolded by terramantle. Replace the placeholder owner.\n");
    s.push_str("# Docs: https://docs.github.com/articles/about-code-owners\n");
    s.push_str("* @your-org/platform\n");
    GeneratedFile {
        path: PathBuf::from(path),
        content: s,
        template: "codeowners".to_string(),
        template_version: TEMPLATE_VERSION,
        class: FileClass::Managed,
    }
}

// ---- Skeleton (scaffold-once) ---------------------------------------------

fn skeleton(m: &Manifest) -> Vec<GeneratedFile> {
    // Poly = one artefact at the repo root; mono = an example under the discovery dir.
    let base = match m.structure {
        Structure::Poly => String::new(),
        Structure::Mono => match m.artefact {
            Artefact::Modules => "modules/example/".to_string(),
            Artefact::Workspaces => "environments/example/".to_string(),
        },
    };
    match m.artefact {
        Artefact::Modules => module_skeleton(&base),
        Artefact::Workspaces => workspace_skeleton(m, &base),
    }
}

fn once(path: String, content: String) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        content,
        template: "skeleton".to_string(),
        template_version: TEMPLATE_VERSION,
        class: FileClass::Once,
    }
}

fn module_skeleton(base: &str) -> Vec<GeneratedFile> {
    vec![
        once(
            format!("{base}main.tf"),
            "# Example module — replace with your resources.\n".to_string(),
        ),
        once(
            format!("{base}variables.tf"),
            "variable \"name\" {\n  description = \"Example input.\"\n  type        = string\n}\n"
                .to_string(),
        ),
        once(
            format!("{base}outputs.tf"),
            "output \"name\" {\n  description = \"Example output.\"\n  value       = var.name\n}\n"
                .to_string(),
        ),
        once(
            format!("{base}versions.tf"),
            "terraform {\n  required_version = \">= 1.7\"\n}\n".to_string(),
        ),
        once(
            format!("{base}README.md"),
            "# Example module\n\n<!-- BEGIN_TF_DOCS -->\n<!-- END_TF_DOCS -->\n".to_string(),
        ),
    ]
}

fn workspace_skeleton(m: &Manifest, base: &str) -> Vec<GeneratedFile> {
    let backend = format!(
        "terraform {{\n  backend \"http\" {{\n    # Terramantle HTTP state backend. `terramantle state publish`\n    # uploads this workspace's .terraform.lock.hcl to the registry.\n    address = \"{registry}/state/{org}/example\"\n  }}\n}}\n",
        registry = registry_url(m),
        org = m.org,
    );
    vec![
        once(
            format!("{base}main.tf"),
            "# Example workspace — replace with your configuration.\n".to_string(),
        ),
        once(format!("{base}backend.tf"), backend),
        once(
            format!("{base}versions.tf"),
            "terraform {\n  required_version = \">= 1.7\"\n}\n".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Discovery, Versioning};
    use std::path::Path;

    fn base(structure: Structure, artefact: Artefact, vcs: VcsProvider) -> Manifest {
        Manifest {
            structure,
            artefact,
            org: "acme".into(),
            api_url: None,
            vcs,
            discovery: match structure {
                Structure::Mono => Some(Discovery::default_for(artefact)),
                Structure::Poly => None,
            },
            ci: CiConfig::default(),
        }
    }

    #[test]
    fn render_is_deterministic() {
        let m = base(Structure::Mono, Artefact::Modules, VcsProvider::Github);
        assert_eq!(desired_files(&m), desired_files(&m));
    }

    #[test]
    fn github_modules_pipeline_path_and_header() {
        let m = base(Structure::Poly, Artefact::Modules, VcsProvider::Github);
        let files = desired_files(&m);
        let ci = files
            .iter()
            .find(|f| f.path == Path::new(".github/workflows/terramantle.yml"))
            .unwrap();
        assert_eq!(ci.template, "github/modules");
        assert_eq!(ci.class, FileClass::Managed);
        assert!(ci
            .content
            .starts_with("# managed by terramantle · template=github/modules"));
        assert!(ci.content.contains("id-token: write")); // oidc default
        assert!(ci.content.contains("TERRAMANTLE_ORG: acme"));
        assert!(ci.content.contains("terramantle modules publish --ci"));
        assert!(!ci.content.contains("--sign")); // signing removed
    }

    #[test]
    fn gitlab_workspaces_pipeline_path() {
        let m = base(Structure::Mono, Artefact::Workspaces, VcsProvider::Gitlab);
        let files = desired_files(&m);
        assert!(files
            .iter()
            .any(|f| f.path == Path::new(".gitlab-ci.yml") && f.template == "gitlab/workspaces"));
        // workspaces never sign
        let ci = files
            .iter()
            .find(|f| f.path == Path::new(".gitlab-ci.yml"))
            .unwrap();
        assert!(ci.content.contains("state publish"));
        assert!(!ci.content.contains("--sign"));
    }

    #[test]
    fn features_toggled_off_are_omitted_not_commented() {
        let mut m = base(Structure::Poly, Artefact::Modules, VcsProvider::Github);
        m.ci.lint = false;
        m.ci.security_scan = false;
        m.ci.terraform_docs = false;
        m.ci.tofu = false;
        let files = desired_files(&m);
        let ci = files
            .iter()
            .find(|f| f.path == Path::new(".github/workflows/terramantle.yml"))
            .unwrap();
        assert!(!ci.content.contains("tflint"));
        assert!(!ci.content.contains("scan --fail-on-atrisk"));
        assert!(!ci.content.contains("terraform-docs"));
        assert!(!ci.content.contains("opentofu"));
        assert!(!ci.content.contains("--sign"));
        // no "rendered when" annotation leakage
        assert!(!ci.content.contains("rendered when"));
    }

    #[test]
    fn bot_auth_swaps_oidc_perms_for_secrets() {
        let mut m = base(Structure::Poly, Artefact::Modules, VcsProvider::Github);
        m.ci.auth = CiAuth::Bot;
        let ci = &desired_files(&m)[0];
        assert!(!ci.content.contains("id-token: write"));
        assert!(ci.content.contains("TERRAMANTLE_BOT_CLIENT_ID"));
    }

    #[test]
    fn mono_modules_skeleton_lives_under_discovery_dir() {
        let m = base(Structure::Mono, Artefact::Modules, VcsProvider::Github);
        let files = desired_files(&m);
        assert!(files
            .iter()
            .any(|f| f.path == Path::new("modules/example/main.tf") && f.class == FileClass::Once));
        assert!(files
            .iter()
            .any(|f| f.path == Path::new("modules/example/README.md")));
    }

    #[test]
    fn poly_workspace_skeleton_at_root_with_backend() {
        let m = base(Structure::Poly, Artefact::Workspaces, VcsProvider::Gitlab);
        let files = desired_files(&m);
        let backend = files
            .iter()
            .find(|f| f.path == Path::new("backend.tf"))
            .unwrap();
        assert_eq!(backend.class, FileClass::Once);
        assert!(backend.content.contains("backend \"http\""));
        assert!(backend.content.contains("/state/acme/"));
    }

    #[test]
    fn skeleton_files_have_no_provenance_header() {
        let m = base(Structure::Poly, Artefact::Modules, VcsProvider::Github);
        let files = desired_files(&m);
        let main = files
            .iter()
            .find(|f| f.path == Path::new("main.tf"))
            .unwrap();
        assert!(!main.content.contains("managed by terramantle"));
    }

    #[test]
    fn gitignore_appends_missing_lines_only() {
        let out = merge_gitignore(Some("node_modules/\n.terraform/\n")).unwrap();
        assert!(out.contains("node_modules/")); // preserved
        assert!(out.contains("*.tfstate")); // added
        assert_eq!(out.matches(".terraform/").count(), 1); // not duplicated
        assert!(out.contains("# terramantle"));
    }

    #[test]
    fn gitignore_noop_when_all_present() {
        let full = GITIGNORE_LINES.join("\n");
        assert_eq!(merge_gitignore(Some(&full)), None);
    }

    #[test]
    fn gitignore_creates_from_nothing() {
        let out = merge_gitignore(None).unwrap();
        for l in GITIGNORE_LINES {
            assert!(out.contains(l));
        }
    }

    #[test]
    fn manual_versioning_still_renders() {
        // versioning is a publish-time concern; the pipeline renders regardless.
        let mut m = base(Structure::Poly, Artefact::Modules, VcsProvider::Github);
        m.ci.versioning = Versioning::Manual;
        assert!(!desired_files(&m).is_empty());
    }
}
