//! Pinned third-party versions and action digests used by the generated CI.
//!
//! Everything a generated pipeline executes is pinned here, in one table, for
//! two reasons:
//!
//! * **Supply chain.** Terramantle's whole pitch is governing what your
//!   infrastructure pulls in. Emitting `uses: actions/checkout@v4` — a mutable
//!   tag — in its own scaffolding would be indefensible. Every action is pinned
//!   to a 40-character commit digest with the tag recorded alongside it as a
//!   comment, and every downloaded binary is checksum-verified against the
//!   release's published sums.
//! * **Reproducibility.** The plan and apply jobs must install the *same*
//!   OpenTofu: a saved plan file is only valid for the binary that produced it.
//!   A floating version would break that contract silently, mid-release.
//!
//! Refreshing a pin is a deliberate commit: bump the version, resolve the new
//! digest, and let the golden-file tests show the diff.

/// An action pinned to an immutable commit digest, carrying its human-readable
/// tag so the rendered YAML stays reviewable.
#[derive(Debug, Clone, Copy)]
pub struct PinnedAction {
    pub repo: &'static str,
    pub tag: &'static str,
    pub sha: &'static str,
}

impl PinnedAction {
    /// `owner/repo@<sha> # <tag>` — the form every `uses:` takes.
    pub fn uses(&self) -> String {
        format!("{}@{} # {}", self.repo, self.sha, self.tag)
    }
}

pub const CHECKOUT: PinnedAction = PinnedAction {
    repo: "actions/checkout",
    tag: "v7.0.1",
    sha: "3d3c42e5aac5ba805825da76410c181273ba90b1",
};

pub const UPLOAD_ARTIFACT: PinnedAction = PinnedAction {
    repo: "actions/upload-artifact",
    tag: "v7.0.2",
    sha: "cf430e030ddbb5b0abf93d22962f4752f3646cd9",
};

pub const DOWNLOAD_ARTIFACT: PinnedAction = PinnedAction {
    repo: "actions/download-artifact",
    tag: "v8.0.2",
    sha: "9000827ccba6bdab643e8b6fd33ac0654aef8333",
};

pub const GITHUB_SCRIPT: PinnedAction = PinnedAction {
    repo: "actions/github-script",
    tag: "v9.0.0",
    sha: "3a2844b7e9c422d3c10d287c895573f7108da1b3",
};

pub const SETUP_OPENTOFU: PinnedAction = PinnedAction {
    repo: "opentofu/setup-opentofu",
    tag: "v2.0.2",
    sha: "a1320f892987e89d278cc92dc5adc984fb93aca4",
};

/// Note `terramate-action@v3` removed its fallback to "latest": without either
/// an explicit `version:` input or a committed `.tool-versions` it calls
/// `exit 1`. Every rendered usage therefore passes `version:`.
pub const TERRAMATE_ACTION: PinnedAction = PinnedAction {
    repo: "terramate-io/terramate-action",
    tag: "v3.3.0",
    sha: "1cc78a548efefaab7762f4d06f51aa6de06ce201",
};

pub const SETUP_TFLINT: PinnedAction = PinnedAction {
    repo: "terraform-linters/setup-tflint",
    tag: "v6.3.2",
    sha: "6ffdbaa3be476b3431f7275c26966ef32ff60337",
};

/// tflint release the GitLab installer downloads. Its release assets are
/// `tflint_linux_amd64.zip` + `checksums.txt` (no version in the filename).
pub const TFLINT_VERSION: &str = "0.65.0";

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[PinnedAction] = &[
        CHECKOUT,
        UPLOAD_ARTIFACT,
        DOWNLOAD_ARTIFACT,
        GITHUB_SCRIPT,
        SETUP_OPENTOFU,
        TERRAMATE_ACTION,
        SETUP_TFLINT,
    ];

    /// A mutable tag in a `uses:` is the failure this module exists to prevent,
    /// so assert the shape rather than trusting review.
    #[test]
    fn every_action_is_pinned_to_a_full_commit_digest() {
        for a in ALL {
            assert_eq!(a.sha.len(), 40, "{} is not a full digest", a.repo);
            assert!(
                a.sha
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{} digest is not lowercase hex",
                a.repo
            );
            assert!(a.tag.starts_with('v'), "{} tag should be vX.Y.Z", a.repo);
        }
    }

    #[test]
    fn uses_renders_digest_with_the_tag_as_a_comment() {
        assert_eq!(
            CHECKOUT.uses(),
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"
        );
    }
}
