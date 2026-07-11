mod agent;
mod backup;
mod cli;
mod error;
mod file_write;
mod identity;
mod opencode;
mod output;
mod target;
mod update;

use std::io::{self, IsTerminal};

use crate::{
    cli::run,
    output::{OutputKind, format_output},
};

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            format_output(
                OutputKind::Error,
                &format!("error: {error}"),
                io::stderr().is_terminal(),
            ),
        );
        std::process::exit(1);
    }
}
