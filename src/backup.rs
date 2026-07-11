use std::{
    fs::{File, OpenOptions},
    io::{self, Seek, SeekFrom},
    path::Path,
    thread,
    time::Duration,
};

use time::{OffsetDateTime, macros::format_description};

use crate::error::CliResult;

const BACKUP_TIMESTAMP_COLLISION_SLEEP_IN_SECS: u64 = 1;

pub fn backup_existing_file(target_file_path: &Path, target_file: &mut File) -> CliResult<()> {
    let file_name = target_file_path
        .file_name()
        .ok_or_else(|| format!("missing file name for {}", target_file_path.display()))?
        .to_string_lossy();
    let mut backup_file = loop {
        let backup_file_name = format!("{file_name}.medotmd.bak-{}", get_timestamp()?);
        let maybe_backup_file_path = target_file_path.with_file_name(backup_file_name);

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(maybe_backup_file_path)
        {
            Ok(backup_file) => break backup_file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                thread::sleep(Duration::from_secs(
                    BACKUP_TIMESTAMP_COLLISION_SLEEP_IN_SECS,
                ));
            }
            Err(error) => return Err(error.into()),
        }
    };

    target_file.seek(SeekFrom::Start(0))?;
    io::copy(target_file, &mut backup_file)?;
    backup_file.set_permissions(target_file.metadata()?.permissions())?;
    backup_file.sync_all()?;

    Ok(())
}

fn get_timestamp() -> CliResult<String> {
    let timestamp_format = format_description!("[year][month][day]-[hour][minute][second]");
    let timestamp = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(timestamp_format)?;

    Ok(timestamp)
}
