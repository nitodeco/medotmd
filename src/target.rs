use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use crate::{
    agent::{
        AGENT_TARGETS, AgentKind, AgentTarget, does_agent_match_filter, get_agent_folder_path,
    },
    backup::backup_existing_file,
    cli::CommandOptions,
    error::CliResult,
    identity::{
        ensure_guidance_file, ensure_identity_file, print_guidance_file_action,
        print_guidance_file_status, print_identity_file_action, print_identity_file_status,
    },
    output::{OutputKind, print_output},
};

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
    ImportMissing(String),
    TargetFileMissing,
    Duplicated(String, usize),
    Unreadable(String),
    Unwritable(String),
}

pub fn install_targets(
    home_path: &Path,
    identity_file_path: &Path,
    guidance_file_path: &Path,
    identity_import_line: &str,
    guidance_import_line: &str,
    command_options: &CommandOptions,
) -> CliResult<()> {
    print_identity_file_action(ensure_identity_file(
        identity_file_path,
        command_options.dry_run,
    )?);
    print_guidance_file_action(ensure_guidance_file(
        guidance_file_path,
        command_options.dry_run,
    )?);

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
        let install_action = install_target_file(
            &target_file_path,
            &[guidance_import_line, identity_import_line],
            command_options.dry_run,
        )?;

        print_install_action(agent_target, install_action);
    }

    Ok(())
}

fn install_target_file(
    target_file_path: &Path,
    import_lines: &[&str],
    is_dry_run: bool,
) -> CliResult<InstallAction> {
    match fs::read_to_string(target_file_path) {
        Ok(existing_content) => {
            if has_exactly_one_import_for_each(&existing_content, import_lines) {
                return Ok(InstallAction::Unchanged);
            }

            if is_dry_run {
                return Ok(InstallAction::WouldModify);
            }

            backup_existing_file(target_file_path)?;
            fs::write(
                target_file_path,
                add_import_lines(&existing_content, import_lines),
            )?;

            Ok(InstallAction::Modified)
        }
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(InstallAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::write(target_file_path, format_import_lines(import_lines))?;

            Ok(InstallAction::Created)
        }
        Err(error) => Err(error.into()),
    }
}

fn print_install_action(agent_target: AgentTarget, install_action: InstallAction) {
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

pub fn uninstall_targets(
    home_path: &Path,
    identity_import_line: &str,
    guidance_import_line: &str,
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
        let uninstall_action = uninstall_target_file(
            &target_file_path,
            &[guidance_import_line, identity_import_line],
            command_options.dry_run,
        )?;

        print_uninstall_action(agent_target, uninstall_action);
    }

    Ok(())
}

fn uninstall_target_file(
    target_file_path: &Path,
    import_lines: &[&str],
    is_dry_run: bool,
) -> CliResult<UninstallAction> {
    match fs::read_to_string(target_file_path) {
        Ok(existing_content) => {
            if !import_lines
                .iter()
                .any(|import_line| count_exact_import_lines(&existing_content, import_line) > 0)
            {
                return Ok(UninstallAction::Unchanged);
            }

            if is_dry_run {
                return Ok(UninstallAction::WouldRemove);
            }

            backup_existing_file(target_file_path)?;
            fs::write(
                target_file_path,
                remove_exact_import_lines(&existing_content, import_lines),
            )?;

            Ok(UninstallAction::Removed)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(UninstallAction::Unchanged),
        Err(error) => Err(error.into()),
    }
}

fn print_uninstall_action(agent_target: AgentTarget, uninstall_action: UninstallAction) {
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

pub fn print_doctor_report(
    home_path: &Path,
    identity_file_path: &Path,
    guidance_file_path: &Path,
    identity_import_line: &str,
    guidance_import_line: &str,
    maybe_agent_kind: Option<AgentKind>,
) -> CliResult<()> {
    print_identity_file_status(identity_file_path)?;
    print_guidance_file_status(guidance_file_path)?;

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
        let target_status = get_target_status(
            &target_file_path,
            &[
                ("AGENT.md", guidance_import_line),
                ("ME.md", identity_import_line),
            ],
        );

        print_target_status(agent_target, target_status);
    }

    Ok(())
}

fn print_target_status(agent_target: AgentTarget, target_status: TargetStatus) {
    match target_status {
        TargetStatus::Installed => {
            print_output(
                OutputKind::Success,
                &format!("{} installed", agent_target.name),
            );
        }
        TargetStatus::ImportMissing(file_name) => {
            print_output(
                OutputKind::Error,
                &format!("{} missing {file_name} import", agent_target.name),
            );
        }
        TargetStatus::TargetFileMissing => {
            print_output(
                OutputKind::Error,
                &format!("{} target file missing", agent_target.name),
            );
        }
        TargetStatus::Duplicated(file_name, count) => {
            print_output(
                OutputKind::Error,
                &format!(
                    "{} duplicated {file_name} import ({count})",
                    agent_target.name
                ),
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

fn get_target_status(target_file_path: &Path, profile_imports: &[(&str, &str)]) -> TargetStatus {
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

    for (file_name, import_line) in profile_imports {
        match count_exact_import_lines(&existing_content, import_line) {
            0 => return TargetStatus::ImportMissing((*file_name).to_string()),
            1 => {}
            count => return TargetStatus::Duplicated((*file_name).to_string(), count),
        }
    }

    TargetStatus::Installed
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

fn count_exact_import_lines(content: &str, import_line: &str) -> usize {
    content.lines().filter(|line| *line == import_line).count()
}

fn has_exactly_one_import_for_each(content: &str, import_lines: &[&str]) -> bool {
    import_lines
        .iter()
        .all(|import_line| count_exact_import_lines(content, import_line) == 1)
}

fn format_import_lines(import_lines: &[&str]) -> String {
    format!("{}\n", import_lines.join("\n"))
}

fn add_import_lines(content: &str, import_lines: &[&str]) -> String {
    format!(
        "{}{}",
        format_import_lines(import_lines),
        remove_exact_import_lines(content, import_lines),
    )
}

fn remove_exact_import_lines(content: &str, import_lines: &[&str]) -> String {
    let retained_lines = content
        .split_inclusive('\n')
        .filter(|line| {
            let normalized_line = line.trim_end_matches('\n').trim_end_matches('\r');

            !import_lines.contains(&normalized_line)
        })
        .collect::<String>();

    if content.ends_with('\n') {
        retained_lines
    } else {
        retained_lines.trim_end_matches('\n').to_string()
    }
}
