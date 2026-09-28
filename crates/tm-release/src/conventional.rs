//! Conventional-Commit → [`BumpLevel`] classification (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! Native parsing (commitizen semantics), so CI needs no Node dependency at
//! publish time: `feat!` / `type(scope)!:` / a `BREAKING CHANGE:` body → Major;
//! `feat:` → Minor; `fix:` → Patch; anything else → None. [`max_bump`] folds a
//! range of commits to the highest level.

use crate::version::BumpLevel;

/// The commitizen breaking-change footer token (§5).
const BREAKING_TOKEN: &str = "BREAKING CHANGE:";

/// Classify a single commit into a [`BumpLevel`] from its `subject` (the `%s`
/// header line) and `body` (`%b`). A breaking marker always wins, then the type
/// keyword decides minor/patch, else `None`.
pub fn classify_commit(subject: &str, body: &str) -> BumpLevel {
    if is_breaking(subject, body) {
        return BumpLevel::Major;
    }
    match commit_type(subject).as_deref() {
        Some("feat") => BumpLevel::Minor,
        Some("fix") => BumpLevel::Patch,
        _ => BumpLevel::None,
    }
}

/// Fold an iterator of per-commit levels to the highest (§5). An empty range
/// (no commits) folds to [`BumpLevel::None`].
pub fn max_bump<I: IntoIterator<Item = BumpLevel>>(levels: I) -> BumpLevel {
    levels.into_iter().max().unwrap_or(BumpLevel::None)
}

/// Whether the commit declares a breaking change: a `!` before the `:` in the
/// header (`feat!:` / `feat(api)!:`), or a `BREAKING CHANGE:` footer in the body.
fn is_breaking(subject: &str, body: &str) -> bool {
    if body.contains(BREAKING_TOKEN) || subject.contains(BREAKING_TOKEN) {
        return true;
    }
    matches!(parse_header(subject), Some((_, true)))
}

/// The lowercased Conventional-Commit type keyword (`feat`, `fix`, `chore`, …),
/// or `None` when the subject is not a conventional header.
fn commit_type(subject: &str) -> Option<String> {
    parse_header(subject).map(|(ty, _)| ty)
}

/// Parse a header into `(type, breaking)`. Returns `None` when there is no
/// `type[...]: ` prefix. `breaking` is true when a `!` immediately precedes the
/// colon.
fn parse_header(subject: &str) -> Option<(String, bool)> {
    let s = subject.trim();
    let colon = s.find(':')?;
    let prefix = &s[..colon];
    let (prefix, breaking) = match prefix.strip_suffix('!') {
        Some(p) => (p, true),
        None => (prefix, false),
    };
    // The type is everything up to an optional `(scope)`.
    let ty = match prefix.find('(') {
        Some(open) => &prefix[..open],
        None => prefix,
    };
    let ty = ty.trim();
    if ty.is_empty() || ty.contains(char::is_whitespace) {
        return None;
    }
    Some((ty.to_ascii_lowercase(), breaking))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feat_is_minor() {
        assert_eq!(classify_commit("feat: add x", ""), BumpLevel::Minor);
        assert_eq!(classify_commit("feat(api): add x", ""), BumpLevel::Minor);
    }

    #[test]
    fn fix_is_patch() {
        assert_eq!(classify_commit("fix: correct y", ""), BumpLevel::Patch);
    }

    #[test]
    fn bang_is_major() {
        assert_eq!(classify_commit("feat!: drop v1", ""), BumpLevel::Major);
        assert_eq!(
            classify_commit("refactor(core)!: rename", ""),
            BumpLevel::Major
        );
    }

    #[test]
    fn breaking_change_body_is_major() {
        assert_eq!(
            classify_commit("fix: tweak", "BREAKING CHANGE: removed flag"),
            BumpLevel::Major
        );
    }

    #[test]
    fn other_types_and_noise_are_none() {
        assert_eq!(classify_commit("chore: deps", ""), BumpLevel::None);
        assert_eq!(classify_commit("docs: readme", ""), BumpLevel::None);
        assert_eq!(classify_commit("just a message", ""), BumpLevel::None);
        assert_eq!(classify_commit("Merge branch 'x'", ""), BumpLevel::None);
    }

    #[test]
    fn max_bump_takes_highest() {
        let commits = [("chore: a", ""), ("fix: b", ""), ("feat: c", "")];
        let level = max_bump(commits.iter().map(|(s, b)| classify_commit(s, b)));
        assert_eq!(level, BumpLevel::Minor);
    }

    #[test]
    fn max_bump_empty_is_none() {
        assert_eq!(max_bump(std::iter::empty()), BumpLevel::None);
    }
}
