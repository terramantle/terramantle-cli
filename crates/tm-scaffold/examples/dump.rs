//! Render every pipeline variant to a directory, for YAML linting and review.
//! `cargo run -p tm-scaffold --example dump -- <outdir>`
use tm_scaffold::{
    desired_files, Artefact, CiConfig, Discovery, Manifest, Structure, Terramate, VcsProvider,
};

fn main() {
    let out = std::env::args().nth(1).expect("usage: dump <outdir>");
    for (structure, artefact, vcs, name) in [
        (
            Structure::Mono,
            Artefact::Workspaces,
            VcsProvider::Github,
            "github-workspaces",
        ),
        (
            Structure::Mono,
            Artefact::Workspaces,
            VcsProvider::Gitlab,
            "gitlab-workspaces",
        ),
        (
            Structure::Mono,
            Artefact::Modules,
            VcsProvider::Github,
            "github-modules",
        ),
        (
            Structure::Mono,
            Artefact::Modules,
            VcsProvider::Gitlab,
            "gitlab-modules",
        ),
    ] {
        let m = Manifest {
            structure,
            artefact,
            org: "acme-demo".into(),
            api_url: None,
            vcs,
            discovery: Some(Discovery::default_for(artefact)),
            terramate: match artefact {
                Artefact::Workspaces => Some(Terramate::default()),
                Artefact::Modules => None,
            },
            ci: CiConfig::default(),
        };
        let dir = std::path::Path::new(&out).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("terramantle.hcl"), m.render()).unwrap();
        for f in desired_files(&m) {
            let p = dir.join(&f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &f.content).unwrap();
        }
        println!("{name}");
    }
}
