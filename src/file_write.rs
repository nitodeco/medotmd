use std::{
    fs::{self, File, Metadata, OpenOptions, Permissions},
    io::{self, ErrorKind, Read, Write},
    path::Path,
};

#[cfg(unix)]
use std::os::unix::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawFd,
};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use tempfile::NamedTempFile;

use crate::{backup::backup_existing_file, error::CliResult};

#[derive(Debug)]
pub(crate) struct FileContentSnapshot(Option<FileSnapshot>);

#[derive(Debug, PartialEq, Eq)]
struct FileSnapshot {
    content: String,
    identity: FileIdentity,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    device_id: u64,
    inode_number: u64,
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    creation_time: u64,
    last_write_time: u64,
    file_size: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, PartialEq, Eq)]
struct FileIdentity;

struct ExistingFile {
    content: String,
    file: File,
    identity: FileIdentity,
    permissions: Permissions,
}

#[cfg(unix)]
struct ParentDirectoryLock {
    _directory: File,
}

#[cfg(not(unix))]
struct ParentDirectoryLock;

impl FileContentSnapshot {
    pub fn content(&self) -> Option<&str> {
        self.0
            .as_ref()
            .map(|file_snapshot| file_snapshot.content.as_str())
    }

    pub fn missing() -> Self {
        Self(None)
    }

    fn matches(&self, maybe_existing_file: Option<&ExistingFile>) -> bool {
        match (&self.0, maybe_existing_file) {
            (None, None) => true,
            (Some(file_snapshot), Some(existing_file)) => {
                file_snapshot.content == existing_file.content
                    && file_snapshot.identity == existing_file.identity
            }
            _ => false,
        }
    }
}

pub(crate) fn read_file_content_snapshot(
    target_file_path: &Path,
) -> CliResult<FileContentSnapshot> {
    Ok(FileContentSnapshot(
        read_existing_file(target_file_path)?.map(|existing_file| FileSnapshot {
            content: existing_file.content,
            identity: existing_file.identity,
        }),
    ))
}

pub(crate) fn replace_file_if_unchanged(
    target_file_path: &Path,
    original_content_snapshot: &FileContentSnapshot,
    next_content: &str,
) -> CliResult<()> {
    replace_file_if_unchanged_with_after_staging_action(
        target_file_path,
        original_content_snapshot,
        next_content,
        || {},
    )
}

fn replace_file_if_unchanged_with_after_staging_action<F>(
    target_file_path: &Path,
    original_content_snapshot: &FileContentSnapshot,
    next_content: &str,
    after_staging_action: F,
) -> CliResult<()>
where
    F: FnOnce(),
{
    let parent_directory_path = target_file_path.parent().ok_or_else(|| {
        format!(
            "missing parent directory for {}",
            target_file_path.display()
        )
    })?;
    let staged_file = stage_file(parent_directory_path, next_content)?;
    after_staging_action();
    let _parent_directory_lock = ParentDirectoryLock::acquire(parent_directory_path)?;
    let maybe_current_file = read_existing_file(target_file_path)?;

    if !original_content_snapshot.matches(maybe_current_file.as_ref()) {
        return Err(format!(
            "{} changed since medotmd read it; retry the command",
            target_file_path.display()
        )
        .into());
    }

    if maybe_current_file
        .as_ref()
        .is_some_and(|current_file| current_file.permissions.readonly())
    {
        return Err(format!("{} is read-only", target_file_path.display()).into());
    }

    if let Some(current_file) = &maybe_current_file {
        staged_file
            .as_file()
            .set_permissions(current_file.permissions.clone())?;
    }

    staged_file.as_file().sync_all()?;

    match maybe_current_file {
        Some(mut current_file) => {
            backup_existing_file(target_file_path, &mut current_file.file)?;
            persist_replacement(staged_file, target_file_path)
        }
        None => persist_new_file(staged_file, target_file_path),
    }
}

fn stage_file(parent_directory_path: &Path, next_content: &str) -> CliResult<NamedTempFile> {
    let mut staged_file = NamedTempFile::new_in(parent_directory_path)?;
    staged_file.write_all(next_content.as_bytes())?;
    staged_file.as_file().sync_all()?;

    Ok(staged_file)
}

impl ParentDirectoryLock {
    #[cfg(unix)]
    fn acquire(parent_directory_path: &Path) -> CliResult<Self> {
        let directory = File::open(parent_directory_path)?;
        let lock_result = unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX) };

        if lock_result == 0 {
            return Ok(Self {
                _directory: directory,
            });
        }

        Err(io::Error::last_os_error().into())
    }

    #[cfg(not(unix))]
    fn acquire(_parent_directory_path: &Path) -> CliResult<Self> {
        Ok(Self)
    }
}

fn read_existing_file(target_file_path: &Path) -> CliResult<Option<ExistingFile>> {
    let target_file_metadata = match fs::symlink_metadata(target_file_path) {
        Ok(target_file_metadata) => target_file_metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };

    if target_file_metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to modify symbolic link {}",
            target_file_path.display()
        )
        .into());
    }

    if !target_file_metadata.is_file() {
        return Err(format!(
            "refusing to modify non-regular file {}",
            target_file_path.display()
        )
        .into());
    }

    let mut target_file = match open_existing_file_without_following_links(target_file_path) {
        Ok(target_file) => target_file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) if is_symbolic_link_error(&error) => {
            return Err(format!(
                "refusing to modify symbolic link {}",
                target_file_path.display()
            )
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    let target_file_metadata = target_file.metadata()?;
    let current_path_metadata = fs::symlink_metadata(target_file_path)?;

    if current_path_metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to modify symbolic link {}",
            target_file_path.display()
        )
        .into());
    }

    if !current_path_metadata.is_file() {
        return Err(format!(
            "refusing to modify non-regular file {}",
            target_file_path.display()
        )
        .into());
    }

    if get_file_identity(&target_file_metadata) != get_file_identity(&current_path_metadata) {
        return Err(format!(
            "{} changed while medotmd read it; retry the command",
            target_file_path.display()
        )
        .into());
    }

    if !target_file_metadata.is_file() {
        return Err(format!(
            "refusing to modify non-regular file {}",
            target_file_path.display()
        )
        .into());
    }

    let mut content = String::new();
    target_file.read_to_string(&mut content)?;

    Ok(Some(ExistingFile {
        content,
        file: target_file,
        identity: get_file_identity(&target_file_metadata),
        permissions: target_file_metadata.permissions(),
    }))
}

#[cfg(unix)]
fn get_file_identity(file_metadata: &Metadata) -> FileIdentity {
    FileIdentity {
        device_id: file_metadata.dev(),
        inode_number: file_metadata.ino(),
    }
}

#[cfg(windows)]
fn get_file_identity(file_metadata: &Metadata) -> FileIdentity {
    FileIdentity {
        creation_time: file_metadata.creation_time(),
        last_write_time: file_metadata.last_write_time(),
        file_size: file_metadata.file_size(),
    }
}

#[cfg(not(any(unix, windows)))]
fn get_file_identity(_file_metadata: &Metadata) -> FileIdentity {
    FileIdentity
}

#[cfg(unix)]
fn open_existing_file_without_following_links(target_file_path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(target_file_path)
}

#[cfg(not(unix))]
fn open_existing_file_without_following_links(target_file_path: &Path) -> io::Result<File> {
    File::open(target_file_path)
}

#[cfg(unix)]
fn is_symbolic_link_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_symbolic_link_error(_error: &io::Error) -> bool {
    false
}

fn persist_replacement(staged_file: NamedTempFile, target_file_path: &Path) -> CliResult<()> {
    staged_file
        .persist(target_file_path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.error.into() })?;

    Ok(())
}

fn persist_new_file(staged_file: NamedTempFile, target_file_path: &Path) -> CliResult<()> {
    staged_file
        .persist_noclobber(target_file_path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.error.into() })?;

    Ok(())
}

#[cfg(test)]
fn replace_file_if_unchanged_after_staging<F>(
    target_file_path: &Path,
    original_content_snapshot: &FileContentSnapshot,
    next_content: &str,
    after_staging_action: F,
) -> CliResult<()>
where
    F: FnOnce(),
{
    replace_file_if_unchanged_with_after_staging_action(
        target_file_path,
        original_content_snapshot,
        next_content,
        after_staging_action,
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Read, path::Path};

    #[cfg(unix)]
    use std::{
        fs::File,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::tempdir;

    use super::{
        read_file_content_snapshot, replace_file_if_unchanged,
        replace_file_if_unchanged_after_staging,
    };

    fn backup_count(folder_path: &Path, file_name: &str) -> usize {
        fs::read_dir(folder_path)
            .expect("folder should be readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{file_name}.medotmd.bak-"))
            })
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn replaces_existing_file_atomically() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");
        let mut original_file = File::open(&target_file_path).expect("target file should open");

        replace_file_if_unchanged(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
        )
        .expect("target file should be replaced");

        let mut original_file_content = String::new();
        original_file
            .read_to_string(&mut original_file_content)
            .expect("original handle should remain readable");

        assert_eq!(original_file_content, "original\n");
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("replacement should be readable"),
            "replacement\n"
        );
        assert_eq!(backup_count(temporary_directory.path(), "AGENTS.md"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_existing_permissions_when_replacing_a_file() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        fs::set_permissions(&target_file_path, fs::Permissions::from_mode(0o640))
            .expect("target permissions should be set");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");

        replace_file_if_unchanged(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
        )
        .expect("target file should be replaced");

        assert_eq!(
            fs::metadata(&target_file_path)
                .expect("target metadata should be readable")
                .permissions()
                .mode()
                & 0o7777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_read_only_existing_file() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        fs::set_permissions(&target_file_path, fs::Permissions::from_mode(0o400))
            .expect("target permissions should be set");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");

        let error = replace_file_if_unchanged(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
        )
        .expect_err("read-only target should be rejected");

        assert!(error.to_string().contains("is read-only"));
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("target should remain readable"),
            "original\n"
        );
        assert_eq!(backup_count(temporary_directory.path(), "AGENTS.md"), 0);
    }

    #[test]
    fn rejects_a_changed_existing_file_without_creating_a_backup() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");
        fs::write(&target_file_path, "concurrent change\n")
            .expect("concurrent content should be written");

        let error = replace_file_if_unchanged(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
        )
        .expect_err("changed target should be rejected");

        assert!(error.to_string().contains("changed since medotmd read it"));
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("target should remain readable"),
            "concurrent change\n"
        );
        assert_eq!(backup_count(temporary_directory.path(), "AGENTS.md"), 0);
    }

    #[test]
    fn rejects_a_change_after_staging_without_creating_a_backup() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");

        let error = replace_file_if_unchanged_after_staging(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
            || {
                fs::write(&target_file_path, "concurrent change\n")
                    .expect("concurrent content should be written");
            },
        )
        .expect_err("changed target should be rejected");

        assert!(error.to_string().contains("changed since medotmd read it"));
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("target should remain readable"),
            "concurrent change\n"
        );
        assert_eq!(backup_count(temporary_directory.path(), "AGENTS.md"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_atomic_replacement_after_staging_without_creating_a_backup() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        let replacement_file_path = temporary_directory.path().join("replacement");
        fs::write(&target_file_path, "original\n").expect("target file should be written");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("target content should be read");

        let error = replace_file_if_unchanged_after_staging(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
            || {
                fs::write(&replacement_file_path, "original\n")
                    .expect("replacement file should be written");
                fs::rename(&replacement_file_path, &target_file_path)
                    .expect("replacement should be atomic");
            },
        )
        .expect_err("replaced target should be rejected");

        assert!(error.to_string().contains("changed since medotmd read it"));
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("target should remain readable"),
            "original\n"
        );
        assert_eq!(backup_count(temporary_directory.path(), "AGENTS.md"), 0);
    }

    #[test]
    fn rejects_a_concurrently_created_file() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        let original_content_snapshot =
            read_file_content_snapshot(&target_file_path).expect("missing target should be read");
        fs::write(&target_file_path, "concurrent creation\n")
            .expect("concurrent target should be created");

        let error = replace_file_if_unchanged(
            &target_file_path,
            &original_content_snapshot,
            "replacement\n",
        )
        .expect_err("concurrently created target should be rejected");

        assert!(error.to_string().contains("changed since medotmd read it"));
        assert_eq!(
            fs::read_to_string(&target_file_path).expect("target should remain readable"),
            "concurrent creation\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let destination_file_path = temporary_directory.path().join("destination");
        let target_file_path = temporary_directory.path().join("AGENTS.md");
        fs::write(&destination_file_path, "destination\n")
            .expect("destination file should be written");
        symlink(&destination_file_path, &target_file_path)
            .expect("symbolic link should be created");

        let error = read_file_content_snapshot(&target_file_path)
            .expect_err("symbolic link should be rejected");

        assert!(
            error
                .to_string()
                .contains("refusing to modify symbolic link")
        );
        assert_eq!(
            fs::read_to_string(&destination_file_path).expect("destination should be readable"),
            "destination\n"
        );
    }
}
