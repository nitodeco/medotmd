use std::path::Path;

use clap::{Parser, Subcommand};

use crate::{
    agent::AgentKind,
    error::CliResult,
    identity::{
        edit_identity_file, get_guidance_file_path, get_home_path, get_identity_file_path,
        get_import_line, print_identity_file,
    },
    output::{OutputKind, print_output},
    target::{install_targets, print_doctor_report, uninstall_targets},
    update::update,
};

#[derive(Parser)]
#[command(name = "medotmd")]
#[command(version)]
#[command(about = "Maintain and register a local ME.md identity prompt")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    Init {
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum)]
        agent: Option<AgentKind>,
    },
    Edit,
    Install {
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum)]
        agent: Option<AgentKind>,
    },
    Uninstall {
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum)]
        agent: Option<AgentKind>,
    },
    Doctor {
        #[arg(long, value_enum)]
        agent: Option<AgentKind>,
    },
    Print,
    Update,
}

pub struct CommandOptions {
    pub dry_run: bool,
    pub maybe_agent_kind: Option<AgentKind>,
}

pub fn run() -> CliResult<()> {
    let cli = Cli::parse();

    if matches!(&cli.command, CliCommand::Update) {
        return update();
    }

    let home_path = get_home_path()?;
    let identity_file_path = get_identity_file_path(&home_path);
    let guidance_file_path = get_guidance_file_path(&home_path);
    let identity_import_line = get_import_line(&identity_file_path);
    let guidance_import_line = get_import_line(&guidance_file_path);

    match cli.command {
        CliCommand::Init { dry_run, agent } => initialize(
            &home_path,
            &identity_file_path,
            &guidance_file_path,
            &identity_import_line,
            &guidance_import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Edit => edit_identity_file(&identity_file_path)?,
        CliCommand::Install { dry_run, agent } => install_targets(
            &home_path,
            &identity_file_path,
            &guidance_file_path,
            &identity_import_line,
            &guidance_import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Uninstall { dry_run, agent } => uninstall_targets(
            &home_path,
            &identity_import_line,
            &guidance_import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Doctor { agent } => print_doctor_report(
            &home_path,
            &identity_file_path,
            &guidance_file_path,
            &identity_import_line,
            &guidance_import_line,
            agent,
        )?,
        CliCommand::Print => print_identity_file(&identity_file_path)?,
        CliCommand::Update => unreachable!(),
    }

    Ok(())
}

fn initialize(
    home_path: &Path,
    identity_file_path: &Path,
    guidance_file_path: &Path,
    identity_import_line: &str,
    guidance_import_line: &str,
    command_options: &CommandOptions,
) -> CliResult<()> {
    print_output(OutputKind::Info, "Initializing medotmd");
    install_targets(
        home_path,
        identity_file_path,
        guidance_file_path,
        identity_import_line,
        guidance_import_line,
        command_options,
    )?;
    println!();
    print_doctor_report(
        home_path,
        identity_file_path,
        guidance_file_path,
        identity_import_line,
        guidance_import_line,
        command_options.maybe_agent_kind,
    )?;

    if command_options.dry_run {
        println!();
        print_output(OutputKind::Success, "Dry run: no files changed");
    }

    Ok(())
}
