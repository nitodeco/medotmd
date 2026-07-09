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

pub enum IdentityFileAction {
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

pub fn get_import_line(identity_file_path: &Path) -> String {
    format!("@{}", identity_file_path.display())
}

pub fn ensure_identity_file(
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

pub fn print_identity_file_action(identity_file_action: IdentityFileAction) {
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

pub fn print_identity_file(identity_file_path: &Path) -> CliResult<()> {
    print!("{}", fs::read_to_string(identity_file_path)?);

    Ok(())
}
