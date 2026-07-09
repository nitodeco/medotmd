use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    io::{self, ErrorKind, IsTerminal},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    thread,
    time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
use time::{OffsetDateTime, macros::format_description};

type CliResult<T> = Result<T, Box<dyn Error>>;

const IDENTITY_FILE_CONTENT: &str = "# Me\n";
const BACKUP_TIMESTAMP_COLLISION_SLEEP_IN_SECS: u64 = 1;
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BLUE: &str = "\x1b[34m";

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
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AgentKind {
    Codex,
    Claude,
    Opencode,
}

#[derive(Clone, Copy)]
struct AgentTarget {
    kind: AgentKind,
    name: &'static str,
    folder_relative_path: &'static str,
    file_name: &'static str,
}

struct CommandOptions {
    dry_run: bool,
    maybe_agent_kind: Option<AgentKind>,
}

enum IdentityFileAction {
    Created,
    Exists,
    WouldCreate,
}

enum InstallAction {
    Created,
    Modified,
    Unchanged,
    WouldCreate,
    WouldModify,
}

enum UninstallAction {
    Removed,
    Unchanged,
    WouldRemove,
}

enum TargetStatus {
    Installed,
    ImportMissing,
    TargetFileMissing,
    Duplicated(usize),
    Unreadable(String),
    Unwritable(String),
}

enum OutputKind {
    Success,
    Warning,
    Error,
    Info,
}

const AGENT_TARGETS: [AgentTarget; 3] = [
    AgentTarget {
        kind: AgentKind::Codex,
        name: "Codex",
        folder_relative_path: ".codex",
        file_name: "AGENTS.md",
    },
    AgentTarget {
        kind: AgentKind::Claude,
        name: "Claude",
        folder_relative_path: ".claude",
        file_name: "CLAUDE.md",
    },
    AgentTarget {
        kind: AgentKind::Opencode,
        name: "OpenCode",
        folder_relative_path: ".config/opencode",
        file_name: "AGENTS.md",
    },
];

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            format_output(
                OutputKind::Error,
                &format!("error: {error}"),
                io::stderr().is_terminal()
            )
        );
        std::process::exit(1);
    }
}

fn run() -> CliResult<()> {
    let cli = Cli::parse();
    let home_path = get_home_path()?;
    let identity_file_path = get_identity_file_path(&home_path);
    let import_line = get_import_line(&identity_file_path);

    match cli.command {
        CliCommand::Init { dry_run, agent } => initialize(
            &home_path,
            &identity_file_path,
            &import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Edit => edit_identity_file(&identity_file_path)?,
        CliCommand::Install { dry_run, agent } => install_targets(
            &home_path,
            &identity_file_path,
            &import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Uninstall { dry_run, agent } => uninstall_targets(
            &home_path,
            &import_line,
            &CommandOptions {
                dry_run,
                maybe_agent_kind: agent,
            },
        )?,
        CliCommand::Doctor { agent } => {
            print_doctor_report(&home_path, &identity_file_path, &import_line, agent)?
        }
        CliCommand::Print => print_identity_file(&identity_file_path)?,
    }

    Ok(())
}

fn get_home_path() -> CliResult<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home_path| !home_path.as_os_str().is_empty())
        .ok_or_else(|| "HOME is not set".into())
}

fn get_identity_file_path(home_path: &Path) -> PathBuf {
    home_path.join(".me").join("ME.md")
}

fn get_import_line(identity_file_path: &Path) -> String {
    format!("@{}", identity_file_path.display())
}

fn initialize(
    home_path: &Path,
    identity_file_path: &Path,
    import_line: &str,
    command_options: &CommandOptions,
) -> CliResult<()> {
    print_output(OutputKind::Info, "Initializing medotmd");
    print_identity_file_action(
        ensure_identity_file(identity_file_path, command_options.dry_run)?,
        identity_file_path,
    );
    install_targets(home_path, identity_file_path, import_line, command_options)?;
    println!();
    print_doctor_report(
        home_path,
        identity_file_path,
        import_line,
        command_options.maybe_agent_kind,
    )?;

    if command_options.dry_run {
        println!();
        print_output(OutputKind::Success, "Dry run: no files changed");
    }

    Ok(())
}

fn ensure_identity_file(
    identity_file_path: &Path,
    is_dry_run: bool,
) -> CliResult<IdentityFileAction> {
    match fs::metadata(identity_file_path) {
        Ok(_) => Ok(IdentityFileAction::Exists),
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(IdentityFileAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent_path) = identity_file_path.parent() {
                fs::create_dir_all(parent_path)?;
            }

            fs::write(identity_file_path, IDENTITY_FILE_CONTENT)?;

            Ok(IdentityFileAction::Created)
        }
        Err(error) => Err(error.into()),
    }
}

fn print_identity_file_action(
    identity_file_action: IdentityFileAction,
    _identity_file_path: &Path,
) {
    match identity_file_action {
        IdentityFileAction::Created => {
            print_output(OutputKind::Success, "ME.md created");
        }
        IdentityFileAction::Exists => {
            print_output(OutputKind::Success, "ME.md exists");
        }
        IdentityFileAction::WouldCreate => {
            print_output(OutputKind::Warning, "ME.md would be created");
        }
    }
}

fn edit_identity_file(identity_file_path: &Path) -> CliResult<()> {
    ensure_identity_file(identity_file_path, false)?;

    let editor = env::var_os("EDITOR").filter(|editor| !editor.is_empty());
    let editor = editor.unwrap_or_else(|| OsString::from("nano"));
    let exit_status = Command::new(editor).arg(identity_file_path).status()?;

    ensure_successful_editor_exit(exit_status)
}

fn ensure_successful_editor_exit(exit_status: ExitStatus) -> CliResult<()> {
    if exit_status.success() {
        return Ok(());
    }

    Err(format!("editor exited with {exit_status}").into())
}

fn install_targets(
    home_path: &Path,
    identity_file_path: &Path,
    import_line: &str,
    command_options: &CommandOptions,
) -> CliResult<()> {
    if !command_options.dry_run {
        ensure_identity_file(identity_file_path, false)?;
    } else if !identity_file_path.exists() {
        print_output(OutputKind::Warning, "ME.md would be created");
    }

    for agent_target in AGENT_TARGETS {
        if !does_agent_match_filter(agent_target, command_options.maybe_agent_kind) {
            continue;
        }

        let folder_path = get_agent_folder_path(home_path, &agent_target);

        if !folder_path.is_dir() {
            print_output(
                OutputKind::Warning,
                &format!("{} skipped, folder missing", agent_target.name),
            );
            continue;
        }

        let target_file_path = folder_path.join(agent_target.file_name);
        let install_action =
            install_target_file(&target_file_path, import_line, command_options.dry_run)?;

        print_install_action(agent_target, &target_file_path, install_action);
    }

    Ok(())
}

fn install_target_file(
    target_file_path: &Path,
    import_line: &str,
    is_dry_run: bool,
) -> CliResult<InstallAction> {
    match fs::read_to_string(target_file_path) {
        Ok(existing_content) => {
            if count_exact_import_lines(&existing_content, import_line) > 0 {
                return Ok(InstallAction::Unchanged);
            }

            if is_dry_run {
                return Ok(InstallAction::WouldModify);
            }

            backup_existing_file(target_file_path)?;
            fs::write(
                target_file_path,
                format!("{import_line}\n{}", existing_content),
            )?;

            Ok(InstallAction::Modified)
        }
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(InstallAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::write(target_file_path, format!("{import_line}\n"))?;

            Ok(InstallAction::Created)
        }
        Err(error) => Err(error.into()),
    }
}

fn print_install_action(
    agent_target: AgentTarget,
    _target_file_path: &Path,
    install_action: InstallAction,
) {
    match install_action {
        InstallAction::Created => {
            print_output(
                OutputKind::Success,
                &format!("{} created", agent_target.name),
            );
        }
        InstallAction::Modified => {
            print_output(
                OutputKind::Success,
                &format!("{} installed", agent_target.name),
            );
        }
        InstallAction::Unchanged => {
            print_output(
                OutputKind::Success,
                &format!("{} already installed", agent_target.name),
            );
        }
        InstallAction::WouldCreate => {
            print_output(
                OutputKind::Warning,
                &format!("{} would be created", agent_target.name),
            );
        }
        InstallAction::WouldModify => {
            print_output(
                OutputKind::Warning,
                &format!("{} would be installed", agent_target.name),
            );
        }
    }
}

fn uninstall_targets(
    home_path: &Path,
    import_line: &str,
    command_options: &CommandOptions,
) -> CliResult<()> {
    for agent_target in AGENT_TARGETS {
        if !does_agent_match_filter(agent_target, command_options.maybe_agent_kind) {
            continue;
        }

        let folder_path = get_agent_folder_path(home_path, &agent_target);

        if !folder_path.is_dir() {
            print_output(
                OutputKind::Warning,
                &format!("{} skipped, folder missing", agent_target.name),
            );
            continue;
        }

        let target_file_path = folder_path.join(agent_target.file_name);
        let uninstall_action =
            uninstall_target_file(&target_file_path, import_line, command_options.dry_run)?;

        print_uninstall_action(agent_target, &target_file_path, uninstall_action);
    }

    Ok(())
}

fn uninstall_target_file(
    target_file_path: &Path,
    import_line: &str,
    is_dry_run: bool,
) -> CliResult<UninstallAction> {
    match fs::read_to_string(target_file_path) {
        Ok(existing_content) => {
            if count_exact_import_lines(&existing_content, import_line) == 0 {
                return Ok(UninstallAction::Unchanged);
            }

            if is_dry_run {
                return Ok(UninstallAction::WouldRemove);
            }

            backup_existing_file(target_file_path)?;
            fs::write(
                target_file_path,
                remove_exact_import_lines(&existing_content, import_line),
            )?;

            Ok(UninstallAction::Removed)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(UninstallAction::Unchanged),
        Err(error) => Err(error.into()),
    }
}

fn print_uninstall_action(
    agent_target: AgentTarget,
    _target_file_path: &Path,
    uninstall_action: UninstallAction,
) {
    match uninstall_action {
        UninstallAction::Removed => {
            print_output(
                OutputKind::Success,
                &format!("{} uninstalled", agent_target.name),
            );
        }
        UninstallAction::Unchanged => {
            print_output(
                OutputKind::Success,
                &format!("{} not installed", agent_target.name),
            );
        }
        UninstallAction::WouldRemove => {
            print_output(
                OutputKind::Warning,
                &format!("{} would be uninstalled", agent_target.name),
            );
        }
    }
}

fn print_doctor_report(
    home_path: &Path,
    identity_file_path: &Path,
    import_line: &str,
    maybe_agent_kind: Option<AgentKind>,
) -> CliResult<()> {
    print_identity_file_status(identity_file_path)?;

    for agent_target in AGENT_TARGETS {
        if !does_agent_match_filter(agent_target, maybe_agent_kind) {
            continue;
        }

        let folder_path = get_agent_folder_path(home_path, &agent_target);

        if !folder_path.is_dir() {
            print_output(
                OutputKind::Warning,
                &format!("{} folder missing", agent_target.name),
            );
            continue;
        }

        let target_file_path = folder_path.join(agent_target.file_name);
        let target_status = get_target_status(&target_file_path, import_line);

        print_target_status(agent_target, &target_file_path, target_status);
    }

    Ok(())
}

fn print_identity_file_status(identity_file_path: &Path) -> CliResult<()> {
    match fs::read_to_string(identity_file_path) {
        Ok(identity_file_content) => {
            if identity_file_content.trim().is_empty() {
                print_output(OutputKind::Warning, "ME.md exists but is empty");
            } else {
                print_output(OutputKind::Success, "ME.md exists");
            }

            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            print_output(OutputKind::Error, "ME.md missing");

            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn print_target_status(
    agent_target: AgentTarget,
    _target_file_path: &Path,
    target_status: TargetStatus,
) {
    match target_status {
        TargetStatus::Installed => {
            print_output(
                OutputKind::Success,
                &format!("{} installed", agent_target.name),
            );
        }
        TargetStatus::ImportMissing => {
            print_output(OutputKind::Error, &format!("{} missing", agent_target.name));
        }
        TargetStatus::TargetFileMissing => {
            print_output(
                OutputKind::Error,
                &format!("{} target file missing", agent_target.name),
            );
        }
        TargetStatus::Duplicated(count) => {
            print_output(
                OutputKind::Error,
                &format!("{} duplicated import ({count})", agent_target.name),
            );
        }
        TargetStatus::Unreadable(message) => {
            print_output(
                OutputKind::Error,
                &format!("{} unreadable ({message})", agent_target.name),
            );
        }
        TargetStatus::Unwritable(message) => {
            print_output(
                OutputKind::Error,
                &format!("{} unwritable ({message})", agent_target.name),
            );
        }
    }
}

fn get_target_status(target_file_path: &Path, import_line: &str) -> TargetStatus {
    let existing_content = match fs::read_to_string(target_file_path) {
        Ok(existing_content) => existing_content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return TargetStatus::TargetFileMissing;
        }
        Err(error) => return TargetStatus::Unreadable(error.to_string()),
    };

    if let Err(error) = assert_writable(target_file_path) {
        return TargetStatus::Unwritable(error.to_string());
    }

    match count_exact_import_lines(&existing_content, import_line) {
        0 => TargetStatus::ImportMissing,
        1 => TargetStatus::Installed,
        count => TargetStatus::Duplicated(count),
    }
}

fn assert_writable(target_file_path: &Path) -> io::Result<()> {
    let permissions = fs::metadata(target_file_path)?.permissions();

    if permissions.readonly() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "file is read-only",
        ));
    }

    Ok(())
}

fn print_identity_file(identity_file_path: &Path) -> CliResult<()> {
    print!("{}", fs::read_to_string(identity_file_path)?);

    Ok(())
}

fn get_agent_folder_path(home_path: &Path, agent_target: &AgentTarget) -> PathBuf {
    home_path.join(agent_target.folder_relative_path)
}

fn does_agent_match_filter(agent_target: AgentTarget, maybe_agent_kind: Option<AgentKind>) -> bool {
    match maybe_agent_kind {
        Some(agent_kind) => agent_target.kind == agent_kind,
        None => true,
    }
}

fn print_output(output_kind: OutputKind, message: &str) {
    println!(
        "{}",
        format_output(output_kind, message, io::stdout().is_terminal())
    );
}

fn format_output(output_kind: OutputKind, message: &str, is_colored: bool) -> String {
    let icon = get_output_icon(&output_kind);

    if !is_colored {
        return format!("{icon} {message}");
    }

    format!(
        "{}{icon}{} {message}",
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

fn count_exact_import_lines(content: &str, import_line: &str) -> usize {
    content.lines().filter(|line| *line == import_line).count()
}

fn remove_exact_import_lines(content: &str, import_line: &str) -> String {
    let retained_lines = content
        .split_inclusive('\n')
        .filter(|line| line.trim_end_matches('\n').trim_end_matches('\r') != import_line)
        .collect::<String>();

    if content.ends_with('\n') {
        retained_lines
    } else {
        retained_lines.trim_end_matches('\n').to_string()
    }
}

fn backup_existing_file(target_file_path: &Path) -> CliResult<()> {
    let file_name = target_file_path
        .file_name()
        .ok_or_else(|| format!("missing file name for {}", target_file_path.display()))?
        .to_string_lossy();
    let backup_file_path = loop {
        let backup_file_name = format!("{file_name}.medotmd.bak-{}", get_timestamp()?);
        let maybe_backup_file_path = target_file_path.with_file_name(backup_file_name);

        if !maybe_backup_file_path.exists() {
            break maybe_backup_file_path;
        }

        thread::sleep(Duration::from_secs(
            BACKUP_TIMESTAMP_COLLISION_SLEEP_IN_SECS,
        ));
    };

    fs::copy(target_file_path, backup_file_path)?;

    Ok(())
}

fn get_timestamp() -> CliResult<String> {
    let timestamp_format = format_description!("[year][month][day]-[hour][minute][second]");
    let timestamp = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(timestamp_format)?;

    Ok(timestamp)
}
