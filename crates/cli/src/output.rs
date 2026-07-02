//! Output rendering (SPEC §6): borderless kubectl-style tables, `-o json|yaml`,
//! and TTY/`NO_COLOR`-aware colour + trust glyphs.
//!
//! Human narration goes to stderr; parseable data goes to stdout, so `-o json`
//! pipes cleanly.

use std::time::{SystemTime, UNIX_EPOCH};

use comfy_table::presets::NOTHING;
use comfy_table::{Cell, ContentArrangement, Table};
use owo_colors::OwoColorize;
use serde::Serialize;
use tm_config::OutputFormat;

/// Whether coloured output is permitted for this invocation. Colour is enabled
/// only when stdout is a TTY, `NO_COLOR` is unset, and `--no-color` was not
/// passed (§6). `supports_color::on` already honours `NO_COLOR` and TTY
/// detection, so this is the single gate the whole crate consults.
pub fn color_enabled(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    supports_color::on(supports_color::Stream::Stdout).is_some()
}

/// Whether unicode trust glyphs (`✓ ▲ · ✕`) are safe to emit, or the ASCII
/// fallback (`OK WARN -- BLOCK`) should be used instead (§6). We fall back when
/// the terminal is non-unicode or output is not a TTY; `NO_COLOR`/`--no-color`
/// implies a plain pipe too, so we treat "no colour" as "no glyphs" as well —
/// this keeps `-o json | jq` and dumb terminals clean.
pub fn glyphs_enabled(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    // A UTF-8 locale + a TTY is the bar for glyphs. Reuse the colour gate as the
    // TTY signal (both key off stdout being an interactive terminal).
    let tty = supports_color::on(supports_color::Stream::Stdout).is_some();
    let utf8 = locale_is_utf8();
    tty && utf8
}

/// Best-effort UTF-8 locale detection from `LC_ALL`/`LC_CTYPE`/`LANG`.
fn locale_is_utf8() -> bool {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                return v.to_ascii_lowercase().contains("utf-8")
                    || v.to_ascii_lowercase().contains("utf8");
            }
        }
    }
    // No locale vars set at all: assume a modern UTF-8 terminal.
    true
}

/// The rendering style flags a command threads through to its table builders.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Colour permitted (TTY + no `NO_COLOR`/`--no-color`).
    pub color: bool,
    /// Unicode glyphs permitted (else ASCII fallback).
    pub glyphs: bool,
}

impl Style {
    /// Derive the style from the `--no-color` flag + ambient TTY/locale.
    pub fn detect(no_color_flag: bool) -> Self {
        Self {
            color: color_enabled(no_color_flag),
            glyphs: glyphs_enabled(no_color_flag),
        }
    }

    /// A forced-plain style: no colour, ASCII glyphs. Used by golden tests so a
    /// render is deterministic regardless of the test environment.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn plain() -> Self {
        Self {
            color: false,
            glyphs: false,
        }
    }
}

/// A provider's derived Trust Seal verdict (§7). Provider-level, not per-version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustVerdict {
    /// Scan rule exists and reports zero critical/high/vuln counts.
    Trusted,
    /// Scan rule exists and reports at least one critical/high/vuln.
    AtRisk,
    /// No overview/scan-rule row for this provider — in use, never scanned.
    Unscanned,
}

impl TrustVerdict {
    /// The machine label (used in `-o json` and as the ASCII fallback stem).
    pub fn label(self) -> &'static str {
        match self {
            TrustVerdict::Trusted => "trusted",
            TrustVerdict::AtRisk => "at-risk",
            TrustVerdict::Unscanned => "unscanned",
        }
    }

    /// The unicode glyph for this verdict (§6: `✓ ▲ ·`).
    fn glyph(self) -> &'static str {
        match self {
            TrustVerdict::Trusted => "✓",
            TrustVerdict::AtRisk => "▲",
            TrustVerdict::Unscanned => "·",
        }
    }

    /// The ASCII fallback stem (§6: `OK / WARN / --`).
    fn ascii(self) -> &'static str {
        match self {
            TrustVerdict::Trusted => "OK",
            TrustVerdict::AtRisk => "WARN",
            TrustVerdict::Unscanned => "--",
        }
    }

    /// Render the verdict cell: `<glyph> <label>` (or `<ascii> <label>`),
    /// coloured when the style permits. Green = trusted, yellow = at-risk,
    /// dim = unscanned.
    pub fn render(self, style: Style) -> String {
        let mark = if style.glyphs {
            self.glyph()
        } else {
            self.ascii()
        };
        let text = format!("{mark} {}", self.label());
        if !style.color {
            return text;
        }
        match self {
            TrustVerdict::Trusted => text.green().to_string(),
            TrustVerdict::AtRisk => text.yellow().to_string(),
            TrustVerdict::Unscanned => text.dimmed().to_string(),
        }
    }
}

/// Render a boolean "outdated" flag as a glyph column (▲ when outdated, · when
/// current), matching the trust-glyph vocabulary (§7 providers ls OUTDATED).
pub fn outdated_glyph(outdated: bool, style: Style) -> String {
    let (mark, ascii, colored): (&str, &str, fn(&str) -> String) = if outdated {
        ("▲", "WARN", |s| s.yellow().to_string())
    } else {
        ("·", "--", |s| s.dimmed().to_string())
    };
    let text = if style.glyphs { mark } else { ascii };
    if style.color {
        colored(text)
    } else {
        text.to_string()
    }
}

/// A simple borderless table: CAPS headers, left-aligned, dynamic width.
pub struct TableView {
    table: Table,
}

impl TableView {
    pub fn new<H: IntoIterator<Item = S>, S: Into<String>>(headers: H) -> Self {
        let mut table = Table::new();
        table
            .load_preset(NOTHING)
            .set_content_arrangement(ContentArrangement::Dynamic);
        table.set_header(
            headers
                .into_iter()
                .map(|h| Cell::new(h.into().to_uppercase())),
        );
        Self { table }
    }

    pub fn row<R: IntoIterator<Item = S>, S: Into<String>>(&mut self, cells: R) -> &mut Self {
        self.table
            .add_row(cells.into_iter().map(|c| Cell::new(c.into())));
        self
    }

    pub fn render(&self) -> String {
        self.table.to_string()
    }
}

/// Serialize a value as JSON to stdout (§6, stable keys for `jq`).
pub fn print_json<T: Serialize>(value: &T) -> Result<(), Box<dyn std::error::Error>> {
    let s = serde_json::to_string_pretty(value)?;
    println!("{s}");
    Ok(())
}

/// Serialize a value as YAML to stdout (§6).
pub fn print_yaml<T: Serialize>(value: &T) -> Result<(), Box<dyn std::error::Error>> {
    let s = serde_yaml::to_string(value)?;
    print!("{s}");
    Ok(())
}

/// Render a serializable value in the requested machine format, returning
/// `false` for `table`/`wide` so the caller can fall back to a custom table.
pub fn print_structured<T: Serialize>(
    value: &T,
    format: OutputFormat,
) -> Result<bool, Box<dyn std::error::Error>> {
    match format {
        OutputFormat::Json => {
            print_json(value)?;
            Ok(true)
        }
        OutputFormat::Yaml => {
            print_yaml(value)?;
            Ok(true)
        }
        OutputFormat::Table | OutputFormat::Wide => Ok(false),
    }
}

/// Format an epoch timestamp as a coarse relative age ("3d ago", "just now").
///
/// Timestamps in the API are seconds since the Unix epoch, but some tables carry
/// millisecond values; we auto-detect (`|ts| > 1e12` → milliseconds) so both
/// render sensibly. A future timestamp reads "just now".
///
/// Part of the shared output surface; the state-read slice's `versions` table
/// ("2h ago", §6) is its first non-test consumer.
#[cfg_attr(not(test), allow(dead_code))]
pub fn relative_time(ts: i64) -> String {
    let secs = if ts.abs() > 1_000_000_000_000 {
        ts / 1000
    } else {
        ts
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = now - secs;
    if delta <= 0 {
        return "just now".to_string();
    }
    let (n, unit) = if delta < 60 {
        (delta, "s")
    } else if delta < 3600 {
        (delta / 60, "m")
    } else if delta < 86_400 {
        (delta / 3600, "h")
    } else if delta < 2_592_000 {
        (delta / 86_400, "d")
    } else if delta < 31_536_000 {
        (delta / 2_592_000, "mo")
    } else {
        (delta / 31_536_000, "y")
    };
    format!("{n}{unit} ago")
}

/// The em dash used for empty cells across the CLI (§6 mockups).
pub const DASH: &str = "—";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_labels_are_stable() {
        assert_eq!(TrustVerdict::Trusted.label(), "trusted");
        assert_eq!(TrustVerdict::AtRisk.label(), "at-risk");
        assert_eq!(TrustVerdict::Unscanned.label(), "unscanned");
    }

    #[test]
    fn verdict_renders_glyph_when_enabled() {
        let style = Style {
            color: false,
            glyphs: true,
        };
        assert_eq!(TrustVerdict::Trusted.render(style), "✓ trusted");
        assert_eq!(TrustVerdict::AtRisk.render(style), "▲ at-risk");
        assert_eq!(TrustVerdict::Unscanned.render(style), "· unscanned");
    }

    #[test]
    fn verdict_falls_back_to_ascii() {
        let style = Style::plain();
        assert_eq!(TrustVerdict::Trusted.render(style), "OK trusted");
        assert_eq!(TrustVerdict::AtRisk.render(style), "WARN at-risk");
        assert_eq!(TrustVerdict::Unscanned.render(style), "-- unscanned");
    }

    #[test]
    fn plain_style_never_emits_ansi() {
        // No ESC sequences when colour is off (rubric 4).
        let out = TrustVerdict::AtRisk.render(Style::plain());
        assert!(!out.contains('\u{1b}'), "unexpected ANSI in {out:?}");
    }

    #[test]
    fn outdated_glyph_ascii_fallback() {
        assert_eq!(outdated_glyph(true, Style::plain()), "WARN");
        assert_eq!(outdated_glyph(false, Style::plain()), "--");
    }

    #[test]
    fn relative_time_recent_and_days() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(relative_time(now + 100), "just now");
        assert_eq!(relative_time(now - 3 * 86_400), "3d ago");
        assert_eq!(relative_time(now - 2 * 3600), "2h ago");
    }

    #[test]
    fn relative_time_accepts_millis() {
        let now_ms = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 5 * 86_400)
            * 1000;
        assert_eq!(relative_time(now_ms), "5d ago");
    }

    #[test]
    fn verdict_json_is_kebab() {
        let s = serde_json::to_string(&TrustVerdict::AtRisk).unwrap();
        assert_eq!(s, "\"at-risk\"");
    }
}
