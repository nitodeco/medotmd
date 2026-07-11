use std::{
    env, fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

#[cfg(unix)]
use std::{
    fs::OpenOptions,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
};

use crate::{
    error::CliResult,
    output::{OutputKind, print_output},
};

const IDENTITY_FILE_CONTENT: &str = "# Me\n";
const PROFILE_DIRECTORY_NAME: &str = ".me";
const PROFILE_DIRECTORY_MODE: u32 = 0o700;
const PROFILE_FILE_MODE: u32 = 0o600;
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

#[derive(Clone, Copy)]
pub enum DoctorHealth {
    Healthy,
    Invalid,
}

impl DoctorHealth {
    pub fn combine(self, next_doctor_health: Self) -> Self {
        match (self, next_doctor_health) {
            (Self::Healthy, Self::Healthy) => Self::Healthy,
            _ => Self::Invalid,
        }
    }

    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }
}

pub struct ProfileDirectoryStatus {
    pub doctor_health: DoctorHealth,
    pub can_check_profile_files: bool,
}

pub enum ProfileFileAction {
    Created,
    Exists,
    Secured,
    WouldCreate,
    WouldSecure,
}

pub enum ProfileDirectoryAction {
    Created,
    Exists,
    Secured,
    WouldCreate,
    WouldSecure,
}

pub fn get_home_path() -> CliResult<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home_path| !home_path.as_os_str().is_empty())
        .ok_or_else(|| "HOME is not set".into())
}

pub fn get_identity_file_path(home_path: &Path) -> PathBuf {
    get_profile_directory_path(home_path).join("ME.md")
}

pub fn get_guidance_file_path(home_path: &Path) -> PathBuf {
    get_profile_directory_path(home_path).join("AGENT.md")
}

pub fn get_profile_directory_path(home_path: &Path) -> PathBuf {
    home_path.join(PROFILE_DIRECTORY_NAME)
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

pub fn ensure_profile_directory(
    profile_directory_path: &Path,
    is_dry_run: bool,
) -> CliResult<ProfileDirectoryAction> {
    match fs::symlink_metadata(profile_directory_path) {
        Ok(profile_directory_metadata) => ensure_existing_profile_directory(
            profile_directory_path,
            &profile_directory_metadata,
            is_dry_run,
        ),
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(ProfileDirectoryAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            create_profile_directory(profile_directory_path)?;

            Ok(ProfileDirectoryAction::Created)
        }
        Err(error) => Err(error.into()),
    }
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
    let profile_directory_path = get_profile_directory_path_from_file(profile_file_path)?;
    ensure_profile_directory(profile_directory_path, is_dry_run)?;

    match fs::symlink_metadata(profile_file_path) {
        Ok(profile_file_metadata) => {
            ensure_existing_profile_file(profile_file_path, &profile_file_metadata, is_dry_run)
        }
        Err(error) if error.kind() == ErrorKind::NotFound && is_dry_run => {
            Ok(ProfileFileAction::WouldCreate)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            create_profile_file(profile_file_path, profile_file_content)?;

            Ok(ProfileFileAction::Created)
        }
        Err(error) => Err(error.into()),
    }
}

fn get_profile_directory_path_from_file(profile_file_path: &Path) -> CliResult<&Path> {
    profile_file_path
        .parent()
        .ok_or_else(|| "profile file path has no parent".into())
}

fn ensure_existing_profile_directory(
    profile_directory_path: &Path,
    profile_directory_metadata: &fs::Metadata,
    is_dry_run: bool,
) -> CliResult<ProfileDirectoryAction> {
    ensure_expected_path_type(profile_directory_metadata, true, PROFILE_DIRECTORY_NAME)?;

    if has_private_permissions(profile_directory_metadata, PROFILE_DIRECTORY_MODE) {
        return Ok(ProfileDirectoryAction::Exists);
    }

    if is_dry_run {
        return Ok(ProfileDirectoryAction::WouldSecure);
    }

    set_private_permissions(profile_directory_path, PROFILE_DIRECTORY_MODE, true)?;

    Ok(ProfileDirectoryAction::Secured)
}

fn ensure_existing_profile_file(
    profile_file_path: &Path,
    profile_file_metadata: &fs::Metadata,
    is_dry_run: bool,
) -> CliResult<ProfileFileAction> {
    ensure_expected_path_type(
        profile_file_metadata,
        false,
        &profile_file_path.display().to_string(),
    )?;

    if has_private_permissions(profile_file_metadata, PROFILE_FILE_MODE) {
        return Ok(ProfileFileAction::Exists);
    }

    if is_dry_run {
        return Ok(ProfileFileAction::WouldSecure);
    }

    set_private_permissions(profile_file_path, PROFILE_FILE_MODE, false)?;

    Ok(ProfileFileAction::Secured)
}

fn ensure_expected_path_type(
    profile_path_metadata: &fs::Metadata,
    expects_directory: bool,
    path_name: &str,
) -> CliResult<()> {
    let file_type = profile_path_metadata.file_type();

    if file_type.is_symlink() {
        return Err(format!("{path_name} must not be a symbolic link").into());
    }

    if expects_directory && !file_type.is_dir() {
        return Err(format!("{path_name} must be a directory").into());
    }

    if !expects_directory && !file_type.is_file() {
        return Err(format!("{path_name} must be a regular file").into());
    }

    Ok(())
}

#[cfg(unix)]
fn has_private_permissions(profile_path_metadata: &fs::Metadata, expected_mode: u32) -> bool {
    profile_path_metadata.permissions().mode() & 0o7777 == expected_mode
}

#[cfg(not(unix))]
fn has_private_permissions(_profile_path_metadata: &fs::Metadata, _expected_mode: u32) -> bool {
    true
}

#[cfg(unix)]
fn create_profile_directory(profile_directory_path: &Path) -> CliResult<()> {
    let mut directory_builder = fs::DirBuilder::new();
    directory_builder.mode(PROFILE_DIRECTORY_MODE);
    directory_builder.create(profile_directory_path)?;
    set_private_permissions(profile_directory_path, PROFILE_DIRECTORY_MODE, true)
}

#[cfg(not(unix))]
fn create_profile_directory(profile_directory_path: &Path) -> CliResult<()> {
    fs::create_dir(profile_directory_path)?;

    Ok(())
}

#[cfg(unix)]
fn create_profile_file(profile_file_path: &Path, profile_file_content: &str) -> CliResult<()> {
    let mut profile_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PROFILE_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(profile_file_path)?;
    profile_file.write_all(profile_file_content.as_bytes())?;
    profile_file.set_permissions(fs::Permissions::from_mode(PROFILE_FILE_MODE))?;

    Ok(())
}

#[cfg(not(unix))]
fn create_profile_file(profile_file_path: &Path, profile_file_content: &str) -> CliResult<()> {
    fs::write(profile_file_path, profile_file_content)?;

    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(
    profile_path: &Path,
    expected_mode: u32,
    expects_directory: bool,
) -> CliResult<()> {
    let custom_flags = libc::O_NOFOLLOW
        | if expects_directory {
            libc::O_DIRECTORY
        } else {
            0
        };
    let profile_path_file = OpenOptions::new()
        .read(true)
        .custom_flags(custom_flags)
        .open(profile_path)?;
    profile_path_file.set_permissions(fs::Permissions::from_mode(expected_mode))?;

    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(
    _profile_path: &Path,
    _expected_mode: u32,
    _expects_directory: bool,
) -> CliResult<()> {
    Ok(())
}

pub fn print_identity_file_action(identity_file_action: ProfileFileAction) {
    print_profile_file_action("ME.md", identity_file_action);
}

pub fn print_profile_directory_action(profile_directory_action: ProfileDirectoryAction) {
    match profile_directory_action {
        ProfileDirectoryAction::Created => {
            print_output(OutputKind::Success, ".me directory created");
        }
        ProfileDirectoryAction::Exists => {
            print_output(OutputKind::Success, ".me directory permissions private");
        }
        ProfileDirectoryAction::Secured => {
            print_output(OutputKind::Success, ".me directory permissions secured");
        }
        ProfileDirectoryAction::WouldCreate => {
            print_output(OutputKind::Warning, ".me directory would be created");
        }
        ProfileDirectoryAction::WouldSecure => {
            print_output(
                OutputKind::Warning,
                ".me directory permissions would be secured",
            );
        }
    }
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
        ProfileFileAction::Secured => {
            print_output(
                OutputKind::Success,
                &format!("{file_name} permissions secured"),
            );
        }
        ProfileFileAction::WouldCreate => {
            print_output(
                OutputKind::Warning,
                &format!("{file_name} would be created"),
            );
        }
        ProfileFileAction::WouldSecure => {
            print_output(
                OutputKind::Warning,
                &format!("{file_name} permissions would be secured"),
            );
        }
    }
}

pub fn edit_identity_file(identity_file_path: &Path) -> CliResult<()> {
    let editor_command = get_editor_command()?;
    ensure_identity_file(identity_file_path, false)?;

    let exit_status = Command::new(editor_command.executable)
        .args(editor_command.arguments)
        .arg(identity_file_path)
        .status()?;

    ensure_successful_editor_exit(exit_status)
}

struct EditorCommand {
    executable: String,
    arguments: Vec<String>,
}

fn get_editor_command() -> CliResult<EditorCommand> {
    let (editor_variable_name, editor_command) = get_configured_editor_command();
    let editor_command = editor_command.into_string().map_err(|_| {
        format!("{editor_variable_name} must contain valid Unicode editor command text")
    })?;
    let editor_parts = shell_words::split(&editor_command).map_err(|error| {
        format!("{editor_variable_name} is not a valid editor command: {error}")
    })?;
    let (editor_executable, editor_arguments) = editor_parts
        .split_first()
        .ok_or_else(|| format!("{editor_variable_name} must contain an editor executable"))?;

    if editor_executable.is_empty() {
        return Err(format!("{editor_variable_name} must contain an editor executable").into());
    }

    Ok(EditorCommand {
        executable: editor_executable.to_owned(),
        arguments: editor_arguments.to_owned(),
    })
}

fn get_configured_editor_command() -> (&'static str, std::ffi::OsString) {
    get_non_empty_environment_variable("VISUAL")
        .map(|editor_command| ("VISUAL", editor_command))
        .or_else(|| {
            get_non_empty_environment_variable("EDITOR")
                .map(|editor_command| ("EDITOR", editor_command))
        })
        .unwrap_or_else(|| ("default editor", "nano".into()))
}

fn get_non_empty_environment_variable(variable_name: &str) -> Option<std::ffi::OsString> {
    env::var_os(variable_name).filter(|editor_command| !editor_command.is_empty())
}

fn ensure_successful_editor_exit(exit_status: ExitStatus) -> CliResult<()> {
    if exit_status.success() {
        return Ok(());
    }

    Err(format!("editor exited with {exit_status}").into())
}

pub fn print_identity_file_status(identity_file_path: &Path) -> DoctorHealth {
    print_profile_file_status(identity_file_path, "ME.md")
}

pub fn print_profile_directory_status(profile_directory_path: &Path) -> ProfileDirectoryStatus {
    let profile_directory_metadata = match fs::symlink_metadata(profile_directory_path) {
        Ok(profile_directory_metadata) => profile_directory_metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            print_output(OutputKind::Error, ".me directory missing");

            return ProfileDirectoryStatus {
                doctor_health: DoctorHealth::Invalid,
                can_check_profile_files: true,
            };
        }
        Err(error) => {
            print_output(
                OutputKind::Error,
                &format!(".me directory unreadable ({error})"),
            );

            return ProfileDirectoryStatus {
                doctor_health: DoctorHealth::Invalid,
                can_check_profile_files: false,
            };
        }
    };

    let file_type = profile_directory_metadata.file_type();

    if file_type.is_symlink() {
        print_output(OutputKind::Error, ".me directory is a symbolic link");

        return ProfileDirectoryStatus {
            doctor_health: DoctorHealth::Invalid,
            can_check_profile_files: false,
        };
    }

    if !file_type.is_dir() {
        print_output(OutputKind::Error, ".me path is not a directory");

        return ProfileDirectoryStatus {
            doctor_health: DoctorHealth::Invalid,
            can_check_profile_files: false,
        };
    }

    if !has_private_permissions(&profile_directory_metadata, PROFILE_DIRECTORY_MODE) {
        print_insecure_permissions(
            ".me directory",
            &profile_directory_metadata,
            PROFILE_DIRECTORY_MODE,
        );

        return ProfileDirectoryStatus {
            doctor_health: DoctorHealth::Invalid,
            can_check_profile_files: true,
        };
    }

    print_output(OutputKind::Success, ".me directory permissions private");

    ProfileDirectoryStatus {
        doctor_health: DoctorHealth::Healthy,
        can_check_profile_files: true,
    }
}

pub fn print_guidance_file_status(guidance_file_path: &Path) -> DoctorHealth {
    print_profile_file_status(guidance_file_path, "AGENT.md")
}

fn print_profile_file_status(profile_file_path: &Path, file_name: &str) -> DoctorHealth {
    let profile_file_metadata = match fs::symlink_metadata(profile_file_path) {
        Ok(profile_file_metadata) => profile_file_metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            print_output(OutputKind::Error, &format!("{file_name} missing"));

            return DoctorHealth::Invalid;
        }
        Err(error) => {
            print_output(
                OutputKind::Error,
                &format!("{file_name} unreadable ({error})"),
            );

            return DoctorHealth::Invalid;
        }
    };
    let file_type = profile_file_metadata.file_type();

    if file_type.is_symlink() {
        print_output(
            OutputKind::Error,
            &format!("{file_name} is a symbolic link"),
        );

        return DoctorHealth::Invalid;
    }

    if !file_type.is_file() {
        print_output(
            OutputKind::Error,
            &format!("{file_name} is not a regular file"),
        );

        return DoctorHealth::Invalid;
    }

    let doctor_health = if has_private_permissions(&profile_file_metadata, PROFILE_FILE_MODE) {
        DoctorHealth::Healthy
    } else {
        print_insecure_permissions(file_name, &profile_file_metadata, PROFILE_FILE_MODE);

        DoctorHealth::Invalid
    };

    match read_profile_file(profile_file_path) {
        Ok(profile_file_content) => {
            if profile_file_content.trim().is_empty() {
                print_output(
                    OutputKind::Warning,
                    &format!("{file_name} exists but is empty"),
                );
            } else {
                print_output(OutputKind::Success, &format!("{file_name} exists"));
            }

            doctor_health
        }
        Err(error) => {
            print_output(
                OutputKind::Error,
                &format!("{file_name} unreadable ({error})"),
            );

            DoctorHealth::Invalid
        }
    }
}

fn read_profile_file(profile_file_path: &Path) -> CliResult<String> {
    let profile_file_metadata = fs::symlink_metadata(profile_file_path)?;
    ensure_expected_path_type(
        &profile_file_metadata,
        false,
        &profile_file_path.display().to_string(),
    )?;

    read_profile_file_contents(profile_file_path).map_err(Into::into)
}

#[cfg(unix)]
fn read_profile_file_contents(profile_file_path: &Path) -> Result<String, std::io::Error> {
    let mut profile_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(profile_file_path)?;
    let mut profile_file_content = String::new();
    profile_file.read_to_string(&mut profile_file_content)?;

    Ok(profile_file_content)
}

#[cfg(not(unix))]
fn read_profile_file_contents(profile_file_path: &Path) -> Result<String, std::io::Error> {
    fs::read_to_string(profile_file_path)
}

#[cfg(unix)]
fn print_insecure_permissions(
    path_name: &str,
    profile_path_metadata: &fs::Metadata,
    expected_mode: u32,
) {
    let actual_mode = profile_path_metadata.permissions().mode() & 0o7777;

    print_output(
        OutputKind::Error,
        &format!(
            "{path_name} permissions insecure (expected {expected_mode:04o}, found {actual_mode:04o})"
        ),
    );
}

#[cfg(not(unix))]
fn print_insecure_permissions(
    _path_name: &str,
    _profile_path_metadata: &fs::Metadata,
    _expected_mode: u32,
) {
}

pub fn print_identity_file(identity_file_path: &Path) -> CliResult<()> {
    print!("{}", read_profile_file(identity_file_path)?);

    Ok(())
}
