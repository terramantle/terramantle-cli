//! Terramantle CLI entry point (SPEC §3, §9).

mod auth;
mod cli;
mod commands;
mod confirm;
mod discovery;
mod lock;
mod output;
mod publish;
mod scaffold;
mod state;

use clap::Parser;

use cli::Cli;

/// Restore the default SIGPIPE disposition so `terramantle … | head` exits
/// quietly instead of panicking on the broken pipe (Rust ignores SIGPIPE by
/// default, which turns pipe closure into an Err every `println!` unwraps).
#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    reset_sigpipe();
    let cli = Cli::parse();
    match commands::dispatch(&cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            // Human narration → stderr (§6). Auth errors keep their §9 exit (5);
            // anything else is the generic 1.
            eprintln!("error: {err}");
            let code = if err.is::<tm_auth::AuthError>() { 5 } else { 1 };
            std::process::exit(code);
        }
    }
}
