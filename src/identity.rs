use std::{
    env,
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use crate::{
    error::CliResult,
    output::{OutputKind, print_output},
};

const IDENTITY_FILE_CONTENT: &str = "# Me\n";
const GUIDANCE_FILE_CONTENT: &str = r#"# Profile maintenance

Treat the accompanying ME.md as the canonical profile for this user.

- Do not inspect or evaluate ME.md at the start of a task or during active work.
- Never interrupt, delay, or reprioritize the user's current request for a profile update.
- Only after the current request is complete, consider a change when the user explicitly stated one clear, durable personal fact in this conversation.
- If there is a candidate, append one concise proposal with the exact intended ME.md change to the completed response.
- If there is no clear candidate, do not mention ME.md.
- Edit ME.md only after the user explicitly approves the proposed change.

Do not infer facts or propose volatile, ambiguous, or sensitive details. An explicit request to update the profile is the only exception and may be handled immediately.
"#;

pub enum ProfileFileAction {
    Created,
    Exists,
    WouldCreate,
}

pub fn get_home_path() -> CliResult<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home_path| !home_path.as_os_str().is_empty())
        .ok_or_else(|| "HOME is not set".into())
}

pub fn get_identity_file_path(home_path: &Path) -> PathBuf {
    home_path.join(".me").join("ME.md")
}

pub fn get_guidance_file_path(home_path: &Path) -> PathBuf {
    home_path.join(".me").join("AGENT.md")
}

pub fn get_import_line(profile_file_path: &Path) -> String {
    format!("@{}", profile_file_path.display())
}

pub fn ensure_identity_file(
    identity_file_path: &Path,
    is_dry_run: bool,
) -> CliResult<ProfileFileAction> {
    ensure_profile_file(identity_file_path, IDENTITY_FILE_CONTENT, is_dry_run)
}

pub fn ensure_guidance_file(
    guidance_file_path: &Path,
    is_dry_run: bool,
) -> CliResult<ProfileFileAction> {
    ensure_profile_file(guidance_file_path, GUIDANCE_FILE_CONTENT, is_dry_run)
}

fn ensure_profile_file(
    profile_file_path: &Path,
    profile_file_content: &str,
    is_dry_run: bool,
) -> CliResult<ProfileFileAction> {
    match fs::metadata(profile_file_path) {
        Ok(_) => Ok(ProfileFileAction::Exists),
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(ProfileFileAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent_path) = profile_file_path.parent() {
                fs::create_dir_all(parent_path)?;
            }

            fs::write(profile_file_path, profile_file_content)?;

            Ok(ProfileFileAction::Created)
        }
        Err(error) => Err(error.into()),
    }
}

pub fn print_identity_file_action(identity_file_action: ProfileFileAction) {
    print_profile_file_action("ME.md", identity_file_action);
}

pub fn print_guidance_file_action(guidance_file_action: ProfileFileAction) {
    print_profile_file_action("AGENT.md", guidance_file_action);
}

fn print_profile_file_action(file_name: &str, profile_file_action: ProfileFileAction) {
    match profile_file_action {
        ProfileFileAction::Created => {
            print_output(OutputKind::Success, &format!("{file_name} created"));
        }
        ProfileFileAction::Exists => {
            print_output(OutputKind::Success, &format!("{file_name} exists"));
        }
        ProfileFileAction::WouldCreate => {
            print_output(
                OutputKind::Warning,
                &format!("{file_name} would be created"),
            );
        }
    }
}

pub fn edit_identity_file(identity_file_path: &Path) -> CliResult<()> {
    ensure_identity_file(identity_file_path, false)?;

    let maybe_editor = env::var_os("EDITOR").filter(|editor| !editor.is_empty());
    let editor = maybe_editor.unwrap_or_else(|| OsString::from("nano"));
    let exit_status = Command::new(editor).arg(identity_file_path).status()?;

    ensure_successful_editor_exit(exit_status)
}

fn ensure_successful_editor_exit(exit_status: ExitStatus) -> CliResult<()> {
    if exit_status.success() {
        return Ok(());
    }

    Err(format!("editor exited with {exit_status}").into())
}

pub fn print_identity_file_status(identity_file_path: &Path) -> CliResult<()> {
    print_profile_file_status(identity_file_path, "ME.md")
}

pub fn print_guidance_file_status(guidance_file_path: &Path) -> CliResult<()> {
    print_profile_file_status(guidance_file_path, "AGENT.md")
}

fn print_profile_file_status(profile_file_path: &Path, file_name: &str) -> CliResult<()> {
    match fs::read_to_string(profile_file_path) {
        Ok(profile_file_content) => {
            if profile_file_content.trim().is_empty() {
                print_output(
                    OutputKind::Warning,
                    &format!("{file_name} exists but is empty"),
                );
            } else {
                print_output(OutputKind::Success, &format!("{file_name} exists"));
            }

            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            print_output(OutputKind::Error, &format!("{file_name} missing"));

            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub fn print_identity_file(identity_file_path: &Path) -> CliResult<()> {
    print!("{}", fs::read_to_string(identity_file_path)?);

    Ok(())
}
