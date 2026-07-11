use std::{
    env,
    fs::{self, File},
    io::{self, Cursor},
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use flate2::read::GzDecoder;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;
use tempfile::{NamedTempFile, tempdir};

use crate::{
    error::CliResult,
    output::{OutputKind, print_output},
};

const BINARY_FILE_NAME: &str = "medotmd";
const CHECKSUM_LENGTH: usize = 64;
const GITHUB_ACCEPT_HEADER: &str = "application/vnd.github+json";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/nitodeco/medotmd/releases/latest";
const USER_AGENT: &str = "medotmd-updater";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

struct UpdatePlan {
    version: Version,
    archive_file_name: String,
    archive_url: String,
    checksum_url: String,
}

pub fn update() -> CliResult<()> {
    let current_version = parse_version(env!("CARGO_PKG_VERSION"))?;
    let release_target = get_release_target()?;

    print_output(OutputKind::Info, "Checking for medotmd updates");

    let release = fetch_latest_release()?;
    let maybe_update_plan = get_update_plan(&release, &current_version, release_target)?;

    let Some(update_plan) = maybe_update_plan else {
        print_output(
            OutputKind::Success,
            &format!("medotmd v{current_version} is already up to date"),
        );
        return Ok(());
    };

    print_output(
        OutputKind::Info,
        &format!(
            "Updating medotmd from v{current_version} to v{}",
            update_plan.version
        ),
    );

    let archive_contents = download_asset(&update_plan.archive_url)?;
    let checksum_contents = String::from_utf8(download_asset(&update_plan.checksum_url)?)?;
    verify_archive_checksum(
        &archive_contents,
        &checksum_contents,
        &update_plan.archive_file_name,
    )?;

    let temporary_directory = tempdir()?;
    let extracted_binary_path = temporary_directory.path().join(BINARY_FILE_NAME);
    extract_binary(&archive_contents, &extracted_binary_path)?;
    replace_current_binary(&extracted_binary_path, &env::current_exe()?)?;

    print_output(
        OutputKind::Success,
        &format!(
            "Updated medotmd from v{current_version} to v{}",
            update_plan.version
        ),
    );

    Ok(())
}

fn fetch_latest_release() -> CliResult<Release> {
    let mut response = ureq::get(GITHUB_LATEST_RELEASE_URL)
        .header("Accept", GITHUB_ACCEPT_HEADER)
        .header("User-Agent", USER_AGENT)
        .call()?;

    let release = response.body_mut().read_json::<Release>()?;

    if release.draft || release.prerelease {
        return Err(invalid_update("GitHub returned a non-stable release").into());
    }

    Ok(release)
}

fn get_update_plan(
    release: &Release,
    current_version: &Version,
    release_target: &str,
) -> CliResult<Option<UpdatePlan>> {
    let latest_version = parse_version(&release.tag_name)?;

    if latest_version <= *current_version {
        return Ok(None);
    }

    let archive_file_name = format!("medotmd-{release_target}.tar.gz");
    let checksum_file_name = format!("{archive_file_name}.sha256");

    Ok(Some(UpdatePlan {
        version: latest_version,
        archive_url: get_asset_url(release, &archive_file_name)?,
        checksum_url: get_asset_url(release, &checksum_file_name)?,
        archive_file_name,
    }))
}

fn parse_version(version_tag: &str) -> CliResult<Version> {
    Version::parse(version_tag.trim_start_matches('v')).map_err(|error| {
        invalid_update(format!("invalid release version {version_tag:?}: {error}")).into()
    })
}

fn get_release_target() -> CliResult<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        (operating_system, architecture) => Err(invalid_update(format!(
            "unsupported platform: {operating_system} {architecture}"
        ))
        .into()),
    }
}

fn get_asset_url(release: &Release, asset_file_name: &str) -> CliResult<String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == asset_file_name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| {
            invalid_update(format!(
                "release {} is missing {asset_file_name}",
                release.tag_name
            ))
            .into()
        })
}

fn download_asset(asset_url: &str) -> CliResult<Vec<u8>> {
    if !asset_url.starts_with("https://") {
        return Err(invalid_update(format!("release asset URL is not HTTPS: {asset_url}")).into());
    }

    let mut response = ureq::get(asset_url)
        .header("User-Agent", USER_AGENT)
        .call()?;

    Ok(response.body_mut().read_to_vec()?)
}

fn verify_archive_checksum(
    archive_contents: &[u8],
    checksum_contents: &str,
    archive_file_name: &str,
) -> CliResult<()> {
    let expected_checksum = checksum_contents
        .lines()
        .find_map(|checksum_line| {
            let mut fields = checksum_line.split_whitespace();
            let checksum = fields.next()?;
            let asset_file_name = fields.next()?.trim_start_matches('*');

            (asset_file_name == archive_file_name).then_some(checksum)
        })
        .ok_or_else(|| invalid_update(format!("missing checksum for {archive_file_name}")))?;

    if expected_checksum.len() != CHECKSUM_LENGTH
        || !expected_checksum
            .bytes()
            .all(|checksum_byte| checksum_byte.is_ascii_hexdigit())
    {
        return Err(
            invalid_update(format!("invalid SHA-256 checksum for {archive_file_name}")).into(),
        );
    }

    let actual_checksum = get_sha256_checksum(archive_contents);

    if !actual_checksum.eq_ignore_ascii_case(expected_checksum) {
        return Err(invalid_update(format!("checksum mismatch for {archive_file_name}")).into());
    }

    Ok(())
}

fn extract_binary(archive_contents: &[u8], output_binary_path: &Path) -> CliResult<()> {
    let archive_reader = GzDecoder::new(Cursor::new(archive_contents));
    let mut archive = Archive::new(archive_reader);
    let mut is_binary_found = false;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let entry_path = entry.path()?;

        if entry_path != Path::new(BINARY_FILE_NAME) {
            continue;
        }

        if !entry.header().entry_type().is_file() || is_binary_found {
            return Err(
                invalid_update("release archive contains an invalid medotmd binary").into(),
            );
        }

        let mut output_file = File::create(output_binary_path)?;
        io::copy(&mut entry, &mut output_file)?;
        output_file.sync_all()?;
        is_binary_found = true;
    }

    if !is_binary_found {
        return Err(invalid_update("release archive is missing the medotmd binary").into());
    }

    Ok(())
}

fn get_sha256_checksum(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|checksum_byte| format!("{checksum_byte:02x}"))
        .collect()
}

fn replace_current_binary(new_binary_path: &Path, current_binary_path: &Path) -> CliResult<()> {
    let current_binary_parent_path = current_binary_path.parent().ok_or_else(|| {
        invalid_update(format!(
            "current executable has no parent directory: {}",
            current_binary_path.display()
        ))
    })?;
    let mut staged_binary = NamedTempFile::new_in(current_binary_parent_path)?;
    let mut new_binary_file = File::open(new_binary_path)?;

    io::copy(&mut new_binary_file, staged_binary.as_file_mut())?;
    set_executable_permissions(staged_binary.path())?;
    staged_binary.as_file_mut().sync_all()?;
    staged_binary
        .persist(current_binary_path)
        .map_err(|error| error.error)?;

    Ok(())
}

#[cfg(unix)]
fn set_executable_permissions(binary_path: &Path) -> CliResult<()> {
    let mut permissions = fs::metadata(binary_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(binary_path, permissions)?;

    Ok(())
}

#[cfg(not(unix))]
fn set_executable_permissions(_binary_path: &Path) -> CliResult<()> {
    Err(invalid_update("self-updates are only supported on Unix platforms").into())
}

fn invalid_update(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor, path::Path};

    use flate2::{Compression, write::GzEncoder};
    use semver::Version;
    use tar::{Builder, Header};
    use tempfile::tempdir;

    use super::{
        BINARY_FILE_NAME, Release, ReleaseAsset, extract_binary, get_sha256_checksum,
        get_update_plan, replace_current_binary, verify_archive_checksum,
    };

    fn create_release(version: &str, asset_names: &[&str]) -> Release {
        Release {
            tag_name: version.to_owned(),
            draft: false,
            prerelease: false,
            assets: asset_names
                .iter()
                .map(|asset_name| ReleaseAsset {
                    name: (*asset_name).to_owned(),
                    browser_download_url: format!("https://example.com/{asset_name}"),
                })
                .collect(),
        }
    }

    fn create_archive(binary_contents: &[u8]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive_builder = Builder::new(encoder);
        let mut header = Header::new_gnu();

        header.set_size(
            binary_contents
                .len()
                .try_into()
                .expect("fixture should fit in u64"),
        );
        header.set_mode(0o755);
        header.set_cksum();
        archive_builder
            .append_data(&mut header, BINARY_FILE_NAME, Cursor::new(binary_contents))
            .expect("fixture archive should be created");

        archive_builder
            .into_inner()
            .expect("fixture archive should finish")
            .finish()
            .expect("fixture gzip should finish")
    }

    fn get_checksum(archive_contents: &[u8]) -> String {
        format!(
            "{}  medotmd-aarch64-apple-darwin.tar.gz\n",
            get_sha256_checksum(archive_contents)
        )
    }

    #[test]
    fn skips_a_release_that_is_not_newer() {
        let release = create_release("v0.3.0", &[]);
        let current_version = Version::parse("0.3.0").expect("version should parse");

        let maybe_update_plan = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .expect("release should be valid");

        assert!(maybe_update_plan.is_none());
    }

    #[test]
    fn selects_platform_specific_release_assets() {
        let release = create_release(
            "v0.4.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sha256",
            ],
        );
        let current_version = Version::parse("0.3.0").expect("version should parse");

        let update_plan = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .expect("release should be valid")
            .expect("update should be available");

        assert_eq!(
            update_plan.version,
            Version::parse("0.4.0").expect("version should parse")
        );
        assert_eq!(
            update_plan.archive_url,
            "https://example.com/medotmd-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            update_plan.checksum_url,
            "https://example.com/medotmd-aarch64-apple-darwin.tar.gz.sha256"
        );
    }

    #[test]
    fn rejects_a_release_missing_the_checksum_asset() {
        let release = create_release("v0.4.0", &["medotmd-aarch64-apple-darwin.tar.gz"]);
        let current_version = Version::parse("0.3.0").expect("version should parse");

        let error = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .err()
            .expect("release should be rejected");

        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn rejects_an_archive_with_an_invalid_checksum() {
        let archive_contents = create_archive(b"new binary");
        let checksum_contents =
            format!("{}  medotmd-aarch64-apple-darwin.tar.gz\n", "0".repeat(64));

        let error = verify_archive_checksum(
            &archive_contents,
            &checksum_contents,
            "medotmd-aarch64-apple-darwin.tar.gz",
        )
        .expect_err("checksum should be rejected");

        assert!(error.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn extracts_and_replaces_the_current_binary() {
        let archive_contents = create_archive(b"new binary");
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let extracted_binary_path = temporary_directory.path().join(BINARY_FILE_NAME);
        let current_binary_path = temporary_directory.path().join("current-medotmd");

        verify_archive_checksum(
            &archive_contents,
            &get_checksum(&archive_contents),
            "medotmd-aarch64-apple-darwin.tar.gz",
        )
        .expect("checksum should be valid");
        extract_binary(&archive_contents, &extracted_binary_path)
            .expect("binary should be extracted");
        fs::write(&current_binary_path, b"old binary").expect("current binary should be written");
        replace_current_binary(&extracted_binary_path, &current_binary_path)
            .expect("current binary should be replaced");

        assert_eq!(
            fs::read(&current_binary_path).expect("current binary should be readable"),
            b"new binary"
        );
        assert!(Path::new(&current_binary_path).exists());
    }
}
