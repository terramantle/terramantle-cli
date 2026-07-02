//! Output rendering skeleton (SPEC §6): borderless kubectl-style tables,
//! `-o json|yaml`, and TTY/`NO_COLOR`-aware colour.
//!
//! Human narration goes to stderr; parseable data goes to stdout, so `-o json`
//! pipes cleanly.

use comfy_table::presets::NOTHING;
use comfy_table::{Cell, ContentArrangement, Table};
use serde::Serialize;
use tm_config::OutputFormat;

/// Whether coloured output is permitted for this invocation. Colour is enabled
/// only when stdout is a TTY, `NO_COLOR` is unset, and `--no-color` was not
/// passed (§6).
pub fn color_enabled(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    supports_color::on(supports_color::Stream::Stdout).is_some()
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
