//! Render every pipeline variant to a directory, for YAML linting and review.
//!
//! The rendered templates are the product's most externally visible artefact
//! and the hardest to test from Rust alone — a workflow can be valid YAML, pass
//! every string assertion, and still be rejected by GitHub. This dumps the real
//! files so `actionlint`, `shellcheck` and a human can look at them.
//!
//! `cargo run -p tm-scaffold --example dump -- <outdir>`

use tm_scaffold::{
    desired_files, Artefact, CiAuth, CiConfig, Discovery, Manifest, Structure, Terramate,
    VcsProvider,
};

struct Variant {
    artefact: Artefact,
    vcs: VcsProvider,
    auth: CiAuth,
    name: &'static str,
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: dump <outdir>");

    let variants = [
        Variant {
            artefact: Artefact::Workspaces,
            vcs: VcsProvider::Github,
            auth: CiAuth::Oidc,
            name: "github-workspaces",
        },
        Variant {
            artefact: Artefact::Workspaces,
            vcs: VcsProvider::Gitlab,
            auth: CiAuth::Oidc,
            name: "gitlab-workspaces",
        },
        Variant {
            artefact: Artefact::Modules,
            vcs: VcsProvider::Github,
            auth: CiAuth::Oidc,
            name: "github-modules",
        },
        Variant {
            artefact: Artefact::Modules,
            vcs: VcsProvider::Gitlab,
            auth: CiAuth::Oidc,
            name: "gitlab-modules",
        },
        // Bot auth swaps the whole credential path, so it gets its own
        // variants rather than being assumed equivalent.
        Variant {
            artefact: Artefact::Workspaces,
            vcs: VcsProvider::Github,
            auth: CiAuth::Bot,
            name: "github-workspaces-bot",
        },
        Variant {
            artefact: Artefact::Workspaces,
            vcs: VcsProvider::Gitlab,
            auth: CiAuth::Bot,
            name: "gitlab-workspaces-bot",
        },
    ];

    for v in variants {
        let m = Manifest {
            structure: Structure::Mono,
            artefact: v.artefact,
            org: "acme-demo".into(),
            api_url: None,
            vcs: v.vcs,
            discovery: Some(Discovery::default_for(v.artefact)),
            terramate: match v.artefact {
                Artefact::Workspaces => Some(Terramate::default()),
                Artefact::Modules => None,
            },
            ci: CiConfig {
                auth: v.auth,
                ..CiConfig::default()
            },
        };
        let dir = std::path::Path::new(&out).join(v.name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("terramantle.hcl"), m.render()).unwrap();
        for f in desired_files(&m) {
            let p = dir.join(&f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &f.content).unwrap();
        }
        println!("{}", v.name);
    }
}
