use std::io::{self, IsTerminal};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BLUE: &str = "\x1b[34m";

pub enum OutputKind {
    Success,
    Warning,
    Error,
    Info,
}

pub fn print_output(output_kind: OutputKind, message: &str) {
    println!(
        "{}",
        format_output(output_kind, message, io::stdout().is_terminal())
    );
}

pub fn format_output(output_kind: OutputKind, message: &str, is_colored: bool) -> String {
    let output_icon = get_output_icon(&output_kind);

    if !is_colored {
        return format!("{output_icon} {message}");
    }

    format!(
        "{}{output_icon}{} {message}",
        get_output_color(&output_kind),
        ANSI_RESET
    )
}

fn get_output_icon(output_kind: &OutputKind) -> &'static str {
    match output_kind {
        OutputKind::Success => "✓",
        OutputKind::Warning => "!",
        OutputKind::Error => "✗",
        OutputKind::Info => "•",
    }
}

fn get_output_color(output_kind: &OutputKind) -> &'static str {
    match output_kind {
        OutputKind::Success => ANSI_GREEN,
        OutputKind::Warning => ANSI_YELLOW,
        OutputKind::Error => ANSI_RED,
        OutputKind::Info => ANSI_BLUE,
    }
}
