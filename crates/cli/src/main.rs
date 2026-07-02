//! Terramantle CLI entry point (SPEC §3, §9).

mod auth;
mod cli;
mod commands;
mod discovery;
mod output;

use clap::Parser;

use cli::Cli;

fn main() {
    let cli = Cli::parse();
    match commands::dispatch(&cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            // Human narration → stderr (§6). Exit 1 = generic error (§9).
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
