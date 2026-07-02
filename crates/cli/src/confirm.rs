//! Shared confirmation machinery for destructive `state` ops (SPEC §8).
//!
//! [`confirm`] prompts `[y/N]` (defaulting to **No**), echoing the concrete blast
//! radius the caller supplies. `--yes`/`--force` short-circuits to `true` without
//! prompting. **Non-interactive** (stdin not a TTY) without `--yes` **fails
//! closed** with exit 4 — never a silent auto-yes (§8). `--force` never bypasses
//! server authz; the server still gates role/human.
//!
//! The prompt/reader/TTY signal are all injected in the pure core [`confirm_with`]
//! so the logic is unit-tested without a real TTY (rubric 2).

use std::io::{BufRead, IsTerminal, Write};

use crate::output::Style;

/// Exit code when confirmation is required but refused / unavailable (§9 code 4).
pub const EXIT_CONFIRM: i32 = 4;

/// Prompt the operator to confirm a destructive op (§8).
///
/// * `assume_yes` (`--yes`||`--force`) → returns `Ok(true)` without prompting.
/// * Interactive TTY → reads `[y/N]` from stdin, defaulting to No.
/// * Non-interactive without `--yes` → `Err(EXIT_CONFIRM)` with a fail-closed
///   message telling the user to pass `--yes`.
///
/// `prompt` must already carry the concrete blast radius. Narration goes to
/// stderr so `-o json`/stdout stays clean.
pub fn confirm(prompt: &str, assume_yes: bool, _style: Style) -> Result<bool, i32> {
    let is_tty = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut err = std::io::stderr();
    confirm_with(prompt, assume_yes, is_tty, &mut reader, &mut err)
}

/// Pure, injectable core of [`confirm`] (rubric 2): the TTY signal, the input
/// reader, and the narration sink are all parameters, so tests exercise every
/// branch without a real terminal.
pub fn confirm_with<R: BufRead, W: Write>(
    prompt: &str,
    assume_yes: bool,
    is_tty: bool,
    reader: &mut R,
    err: &mut W,
) -> Result<bool, i32> {
    // `--yes`/`--force`: skip the prompt entirely (still subject to server authz).
    if assume_yes {
        return Ok(true);
    }

    // Fail closed in pipelines: no TTY + no --yes ⇒ never assume yes (§8).
    if !is_tty {
        let _ = writeln!(
            err,
            "{prompt}\nconfirmation required; pass --yes (non-interactive stdin)"
        );
        return Err(EXIT_CONFIRM);
    }

    // Interactive: echo the blast radius, read a line, default No.
    let _ = write!(err, "{prompt} [y/N] ");
    let _ = err.flush();
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return Err(EXIT_CONFIRM);
    }
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        prompt: &str,
        assume_yes: bool,
        is_tty: bool,
        input: &str,
    ) -> (Result<bool, i32>, String) {
        let mut reader = std::io::Cursor::new(input.as_bytes().to_vec());
        let mut err: Vec<u8> = Vec::new();
        let res = confirm_with(prompt, assume_yes, is_tty, &mut reader, &mut err);
        (res, String::from_utf8(err).unwrap())
    }

    #[test]
    fn assume_yes_short_circuits_without_prompting() {
        // Even non-interactive: --yes returns true and writes NOTHING to stderr.
        let (res, err) = run("blast radius", true, false, "");
        assert_eq!(res, Ok(true));
        assert!(err.is_empty(), "no prompt/narration when --yes: {err:?}");
    }

    #[test]
    fn assume_yes_ignores_reader_contents() {
        // A stray "n" on stdin must not override an explicit --yes.
        let (res, _err) = run("radius", true, true, "n\n");
        assert_eq!(res, Ok(true));
    }

    #[test]
    fn non_interactive_without_yes_returns_exit_4() {
        let (res, err) = run("PROMOTE prod serial 14", false, false, "");
        assert_eq!(res, Err(EXIT_CONFIRM));
        assert_eq!(EXIT_CONFIRM, 4);
        // The blast radius + the remediation are surfaced.
        assert!(err.contains("PROMOTE prod serial 14"), "{err}");
        assert!(err.contains("confirmation required; pass --yes"), "{err}");
    }

    #[test]
    fn interactive_yes_confirms() {
        let (res, err) = run("radius", false, true, "y\n");
        assert_eq!(res, Ok(true));
        assert!(err.contains("radius"));
        assert!(err.contains("[y/N]"));
    }

    #[test]
    fn interactive_yes_word_confirms() {
        assert_eq!(run("r", false, true, "yes\n").0, Ok(true));
        assert_eq!(run("r", false, true, "YES\n").0, Ok(true));
    }

    #[test]
    fn interactive_default_no_on_empty() {
        // Bare Enter → default No (fail closed on ambiguity).
        assert_eq!(run("r", false, true, "\n").0, Ok(false));
    }

    #[test]
    fn interactive_explicit_no() {
        assert_eq!(run("r", false, true, "n\n").0, Ok(false));
        assert_eq!(run("r", false, true, "no\n").0, Ok(false));
        assert_eq!(run("r", false, true, "garbage\n").0, Ok(false));
    }
}
