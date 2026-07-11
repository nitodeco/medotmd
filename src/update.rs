use std::{
    env,
    fs::{self, File},
    io::{self, Cursor},
    path::Path,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use ed25519_dalek::{Signature, VerifyingKey};
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
const RELEASE_SIGNATURE_LENGTH: usize = 64;
const RELEASE_SIGNING_PUBLIC_KEY: [u8; 32] = [
    250, 240, 170, 46, 208, 147, 114, 188, 21, 168, 145, 105, 251, 54, 250, 248, 165, 41, 33, 154,
    137, 139, 238, 85, 113, 43, 191, 47, 246, 246, 251, 222,
];
const GITHUB_ACCEPT_HEADER: &str = "application/vnd.github+json";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/nitodeco/medotmd/releases/latest";
const USER_AGENT: &str = "medotmd-updater";
const MAX_RELEASE_METADATA_BYTES: u64 = 256 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 8 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 1_024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DECOMPRESSED_ARCHIVE_BYTES: u64 = 40 * 1024 * 1024;
const MAX_EXTRACTED_BINARY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_REDIRECTS: u32 = 3;
const MAX_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const UPDATE_GLOBAL_TIMEOUT: Duration = Duration::from_secs(90);
const UPDATE_CALL_TIMEOUT: Duration = Duration::from_secs(60);
const UPDATE_RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
const UPDATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_BODY_TIMEOUT: Duration = Duration::from_secs(60);

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
    release_tag: String,
    release_target: String,
    archive_file_name: String,
    archive_url: String,
    checksum_url: String,
    signature_url: String,
    manifest_url: String,
    manifest_signature_url: String,
}

struct ReleaseManifest {
    version: String,
    release_tag: String,
    release_target: String,
    archive_file_name: String,
    archive_checksum: String,
}

pub fn update() -> CliResult<()> {
    let current_version = parse_version(env!("CARGO_PKG_VERSION"))?;
    let release_target = get_release_target()?;
    let update_agent = get_update_agent();

    print_output(OutputKind::Info, "Checking for medotmd updates");

    let release = fetch_latest_release(&update_agent)?;
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

    let manifest_contents = download_asset(
        &update_agent,
        &update_plan.manifest_url,
        MAX_MANIFEST_BYTES,
        "release manifest",
    )?;
    let manifest_signature_contents = download_asset(
        &update_agent,
        &update_plan.manifest_signature_url,
        MAX_SIGNATURE_BYTES,
        "release manifest signature",
    )?;
    let release_manifest = verify_release_manifest(
        &manifest_contents,
        &manifest_signature_contents,
        &update_plan,
    )?;
    let archive_contents = download_asset(
        &update_agent,
        &update_plan.archive_url,
        MAX_ARCHIVE_BYTES,
        "release archive",
    )?;
    let checksum_contents = String::from_utf8(download_asset(
        &update_agent,
        &update_plan.checksum_url,
        MAX_CHECKSUM_BYTES,
        "release checksum",
    )?)?;
    let signature_contents = download_asset(
        &update_agent,
        &update_plan.signature_url,
        MAX_SIGNATURE_BYTES,
        "release signature",
    )?;
    verify_manifest_archive_checksum(&archive_contents, &release_manifest)?;
    verify_archive_checksum(
        &archive_contents,
        &checksum_contents,
        &update_plan.archive_file_name,
    )?;
    verify_archive_signature(&archive_contents, &signature_contents)?;

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

fn get_update_agent() -> ureq::Agent {
    let configuration = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .max_response_header_size(MAX_RESPONSE_HEADER_BYTES)
        .timeout_global(Some(UPDATE_GLOBAL_TIMEOUT))
        .timeout_per_call(Some(UPDATE_CALL_TIMEOUT))
        .timeout_resolve(Some(UPDATE_RESOLVE_TIMEOUT))
        .timeout_connect(Some(UPDATE_CONNECT_TIMEOUT))
        .timeout_recv_response(Some(UPDATE_RESPONSE_TIMEOUT))
        .timeout_recv_body(Some(UPDATE_BODY_TIMEOUT))
        .build();

    ureq::Agent::new_with_config(configuration)
}

fn fetch_latest_release(update_agent: &ureq::Agent) -> CliResult<Release> {
    let mut response = update_agent
        .get(GITHUB_LATEST_RELEASE_URL)
        .header("Accept", GITHUB_ACCEPT_HEADER)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| invalid_update(format!("failed to fetch latest release: {error}")))?;

    let maybe_content_length = response
        .headers()
        .get("Content-Length")
        .map(|content_length| content_length.to_str())
        .transpose()
        .map_err(|_| invalid_update("latest release metadata has an invalid Content-Length"))?;
    validate_response_content_length(
        maybe_content_length,
        MAX_RELEASE_METADATA_BYTES,
        "latest release metadata",
    )?;

    let release = response
        .body_mut()
        .with_config()
        .limit(MAX_RELEASE_METADATA_BYTES)
        .read_json::<Release>()
        .map_err(|error| {
            invalid_update(format!("failed to read latest release metadata: {error}"))
        })?;

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
    let signature_file_name = format!("{archive_file_name}.sig");
    let manifest_file_name = format!("medotmd-{release_target}.manifest");
    let manifest_signature_file_name = format!("{manifest_file_name}.sig");

    Ok(Some(UpdatePlan {
        version: latest_version,
        release_tag: release.tag_name.clone(),
        release_target: release_target.to_owned(),
        archive_url: get_asset_url(release, &archive_file_name)?,
        checksum_url: get_asset_url(release, &checksum_file_name)?,
        signature_url: get_asset_url(release, &signature_file_name)?,
        manifest_url: get_asset_url(release, &manifest_file_name)?,
        manifest_signature_url: get_asset_url(release, &manifest_signature_file_name)?,
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

fn download_asset(
    update_agent: &ureq::Agent,
    asset_url: &str,
    maximum_response_size: u64,
    asset_description: &str,
) -> CliResult<Vec<u8>> {
    if !asset_url.starts_with("https://") {
        return Err(invalid_update(format!("release asset URL is not HTTPS: {asset_url}")).into());
    }

    let mut response = update_agent
        .get(asset_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| {
            invalid_update(format!("failed to download {asset_description}: {error}"))
        })?;

    let maybe_content_length = response
        .headers()
        .get("Content-Length")
        .map(|content_length| content_length.to_str())
        .transpose()
        .map_err(|_| {
            invalid_update(format!("{asset_description} has an invalid Content-Length"))
        })?;
    validate_response_content_length(
        maybe_content_length,
        maximum_response_size,
        asset_description,
    )?;

    response
        .body_mut()
        .with_config()
        .limit(maximum_response_size)
        .read_to_vec()
        .map_err(|error| invalid_update(format!("failed to read {asset_description}: {error}")))
        .map_err(Into::into)
}

fn validate_response_content_length(
    maybe_content_length: Option<&str>,
    maximum_response_size: u64,
    response_description: &str,
) -> CliResult<()> {
    let Some(content_length) = maybe_content_length else {
        return Ok(());
    };
    let response_size = content_length.trim().parse::<u64>().map_err(|_| {
        invalid_update(format!(
            "{response_description} has an invalid Content-Length: {content_length:?}"
        ))
    })?;

    if response_size > maximum_response_size {
        return Err(invalid_update(format!(
            "{response_description} is too large ({response_size} bytes; limit is {maximum_response_size} bytes)"
        ))
        .into());
    }

    Ok(())
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

    if !is_valid_sha256_checksum(expected_checksum) {
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

fn verify_archive_signature(archive_contents: &[u8], signature_contents: &[u8]) -> CliResult<()> {
    verify_release_signature_with_public_key(
        archive_contents,
        signature_contents,
        &RELEASE_SIGNING_PUBLIC_KEY,
    )
}

fn verify_release_signature_with_public_key(
    signed_contents: &[u8],
    signature_contents: &[u8],
    public_key_bytes: &[u8; 32],
) -> CliResult<()> {
    if signature_contents.len() != RELEASE_SIGNATURE_LENGTH {
        return Err(invalid_update("release signature has an invalid length").into());
    }

    let signature = Signature::from_slice(signature_contents)
        .map_err(|_| invalid_update("release signature is invalid"))?;
    let verifying_key = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|_| invalid_update("trusted release signing key is invalid"))?;

    verifying_key
        .verify_strict(signed_contents, &signature)
        .map_err(|_| invalid_update("release signature verification failed"))?;

    Ok(())
}

fn verify_release_manifest(
    manifest_contents: &[u8],
    signature_contents: &[u8],
    update_plan: &UpdatePlan,
) -> CliResult<ReleaseManifest> {
    verify_release_manifest_with_public_key(
        manifest_contents,
        signature_contents,
        update_plan,
        &RELEASE_SIGNING_PUBLIC_KEY,
    )
}

fn verify_release_manifest_with_public_key(
    manifest_contents: &[u8],
    signature_contents: &[u8],
    update_plan: &UpdatePlan,
    public_key_bytes: &[u8; 32],
) -> CliResult<ReleaseManifest> {
    verify_release_signature_with_public_key(
        manifest_contents,
        signature_contents,
        public_key_bytes,
    )?;
    let manifest_contents = std::str::from_utf8(manifest_contents)
        .map_err(|_| invalid_update("release manifest is not valid UTF-8"))?;
    let release_manifest = parse_release_manifest(manifest_contents)?;

    if release_manifest.version != update_plan.version.to_string() {
        return Err(
            invalid_update("release manifest version does not match the release tag").into(),
        );
    }

    if release_manifest.release_tag != update_plan.release_tag {
        return Err(
            invalid_update("release manifest tag does not match the GitHub release tag").into(),
        );
    }

    if release_manifest.release_target != update_plan.release_target {
        return Err(invalid_update("release manifest target does not match this platform").into());
    }

    if release_manifest.archive_file_name != update_plan.archive_file_name {
        return Err(
            invalid_update("release manifest archive does not match the release asset").into(),
        );
    }

    if !is_valid_sha256_checksum(&release_manifest.archive_checksum) {
        return Err(invalid_update("release manifest has an invalid SHA-256 checksum").into());
    }

    Ok(release_manifest)
}

fn parse_release_manifest(manifest_contents: &str) -> CliResult<ReleaseManifest> {
    let mut maybe_version = None;
    let mut maybe_release_tag = None;
    let mut maybe_release_target = None;
    let mut maybe_archive_file_name = None;
    let mut maybe_archive_checksum = None;

    for manifest_line in manifest_contents.lines() {
        let Some((field_name, field_value)) = manifest_line.split_once('=') else {
            return Err(invalid_update("release manifest has an invalid field").into());
        };

        if field_value.is_empty() {
            return Err(invalid_update("release manifest has an empty field").into());
        }

        let maybe_previous_value = match field_name {
            "version" => maybe_version.replace(field_value.to_owned()),
            "tag" => maybe_release_tag.replace(field_value.to_owned()),
            "target" => maybe_release_target.replace(field_value.to_owned()),
            "archive" => maybe_archive_file_name.replace(field_value.to_owned()),
            "sha256" => maybe_archive_checksum.replace(field_value.to_owned()),
            _ => return Err(invalid_update("release manifest has an unknown field").into()),
        };

        if maybe_previous_value.is_some() {
            return Err(invalid_update("release manifest has a duplicate field").into());
        }
    }

    let version =
        maybe_version.ok_or_else(|| invalid_update("release manifest is missing version"))?;
    let release_tag =
        maybe_release_tag.ok_or_else(|| invalid_update("release manifest is missing tag"))?;
    let release_target =
        maybe_release_target.ok_or_else(|| invalid_update("release manifest is missing target"))?;
    let archive_file_name = maybe_archive_file_name
        .ok_or_else(|| invalid_update("release manifest is missing archive"))?;
    let archive_checksum = maybe_archive_checksum
        .ok_or_else(|| invalid_update("release manifest is missing SHA-256"))?;

    Ok(ReleaseManifest {
        version,
        release_tag,
        release_target,
        archive_file_name,
        archive_checksum,
    })
}

fn verify_manifest_archive_checksum(
    archive_contents: &[u8],
    release_manifest: &ReleaseManifest,
) -> CliResult<()> {
    let actual_checksum = get_sha256_checksum(archive_contents);

    if actual_checksum != release_manifest.archive_checksum {
        return Err(invalid_update("release archive does not match the signed manifest").into());
    }

    Ok(())
}

fn is_valid_sha256_checksum(checksum: &str) -> bool {
    checksum.len() == CHECKSUM_LENGTH
        && checksum
            .bytes()
            .all(|checksum_byte| checksum_byte.is_ascii_hexdigit())
}

fn extract_binary(archive_contents: &[u8], output_binary_path: &Path) -> CliResult<()> {
    extract_binary_with_limits(
        archive_contents,
        output_binary_path,
        MAX_DECOMPRESSED_ARCHIVE_BYTES,
        MAX_EXTRACTED_BINARY_BYTES,
    )
}

fn extract_binary_with_limits(
    archive_contents: &[u8],
    output_binary_path: &Path,
    maximum_decompressed_archive_size: u64,
    maximum_binary_size: u64,
) -> CliResult<()> {
    let archive_reader = LimitedReader::new(
        GzDecoder::new(Cursor::new(archive_contents)),
        maximum_decompressed_archive_size,
        "release archive decompressed size",
    );
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

        if entry.size() > maximum_binary_size {
            return Err(invalid_update(format!(
                "release archive medotmd binary exceeds {maximum_binary_size} bytes"
            ))
            .into());
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

struct LimitedReader<R> {
    reader: R,
    maximum_bytes: u64,
    consumed_bytes: u64,
    limit_description: &'static str,
}

impl<R> LimitedReader<R> {
    fn new(reader: R, maximum_bytes: u64, limit_description: &'static str) -> Self {
        Self {
            reader,
            maximum_bytes,
            consumed_bytes: 0,
            limit_description,
        }
    }
}

impl<R: io::Read> io::Read for LimitedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let remaining_bytes = self.maximum_bytes.saturating_sub(self.consumed_bytes);

        if remaining_bytes == 0 {
            return Err(io::Error::other(format!(
                "{} exceeds {} bytes",
                self.limit_description, self.maximum_bytes
            )));
        }

        let maximum_read_length = match usize::try_from(remaining_bytes) {
            Ok(remaining_length) => remaining_length.min(buffer.len()),
            Err(_) => buffer.len(),
        };
        let bytes_read = self.reader.read(&mut buffer[..maximum_read_length])?;
        let bytes_read = u64::try_from(bytes_read)
            .map_err(|_| io::Error::other("release archive read length exceeds u64"))?;

        self.consumed_bytes = self
            .consumed_bytes
            .checked_add(bytes_read)
            .ok_or_else(|| io::Error::other("release archive decompressed size overflowed"))?;

        usize::try_from(bytes_read)
            .map_err(|_| io::Error::other("release archive read length exceeds usize"))
    }
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

    use ed25519_dalek::{Signer, SigningKey};
    use flate2::{Compression, write::GzEncoder};
    use semver::Version;
    use tar::{Builder, Header};
    use tempfile::tempdir;

    use super::{
        BINARY_FILE_NAME, MAX_REDIRECTS, MAX_RESPONSE_HEADER_BYTES, Release, ReleaseAsset,
        UPDATE_BODY_TIMEOUT, UPDATE_CALL_TIMEOUT, UPDATE_CONNECT_TIMEOUT, UPDATE_GLOBAL_TIMEOUT,
        UPDATE_RESOLVE_TIMEOUT, UPDATE_RESPONSE_TIMEOUT, extract_binary,
        extract_binary_with_limits, get_sha256_checksum, get_update_agent, get_update_plan,
        replace_current_binary, validate_response_content_length, verify_archive_checksum,
        verify_archive_signature, verify_manifest_archive_checksum,
        verify_release_manifest_with_public_key, verify_release_signature_with_public_key,
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
        create_archive_with_entries(&[(BINARY_FILE_NAME, binary_contents)])
    }

    fn create_archive_with_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive_builder = Builder::new(encoder);

        for (entry_path, entry_contents) in entries {
            let mut header = Header::new_gnu();

            header.set_size(
                entry_contents
                    .len()
                    .try_into()
                    .expect("fixture should fit in u64"),
            );
            header.set_mode(0o755);
            header.set_cksum();
            archive_builder
                .append_data(&mut header, entry_path, Cursor::new(entry_contents))
                .expect("fixture archive should be created");
        }

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

    fn get_signature(archive_contents: &[u8]) -> (Vec<u8>, [u8; 32]) {
        let signing_key = SigningKey::from_bytes(&[42; 32]);

        (
            signing_key.sign(archive_contents).to_bytes().to_vec(),
            signing_key.verifying_key().to_bytes(),
        )
    }

    fn create_manifest(
        version: &str,
        release_tag: &str,
        release_target: &str,
        archive_file_name: &str,
        archive_checksum: &str,
    ) -> String {
        format!(
            "version={version}\ntag={release_tag}\ntarget={release_target}\narchive={archive_file_name}\nsha256={archive_checksum}\n"
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
                "medotmd-aarch64-apple-darwin.tar.gz.sig",
                "medotmd-aarch64-apple-darwin.manifest",
                "medotmd-aarch64-apple-darwin.manifest.sig",
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
        assert_eq!(
            update_plan.signature_url,
            "https://example.com/medotmd-aarch64-apple-darwin.tar.gz.sig"
        );
        assert_eq!(
            update_plan.manifest_url,
            "https://example.com/medotmd-aarch64-apple-darwin.manifest"
        );
        assert_eq!(
            update_plan.manifest_signature_url,
            "https://example.com/medotmd-aarch64-apple-darwin.manifest.sig"
        );
    }

    #[test]
    fn rejects_a_release_missing_the_checksum_asset() {
        let release = create_release(
            "v0.4.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sig",
            ],
        );
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
    fn rejects_a_release_missing_the_signature_asset() {
        let release = create_release(
            "v0.4.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sha256",
            ],
        );
        let current_version = Version::parse("0.3.0").expect("version should parse");

        let error = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .err()
            .expect("release should be rejected");

        assert!(error.to_string().contains(".sig"));
    }

    #[test]
    fn rejects_a_release_missing_the_manifest_asset() {
        let release = create_release(
            "v0.4.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sha256",
                "medotmd-aarch64-apple-darwin.tar.gz.sig",
            ],
        );
        let current_version = Version::parse("0.3.0").expect("version should parse");

        let error = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .err()
            .expect("release should be rejected");

        assert!(error.to_string().contains(".manifest"));
    }

    #[test]
    fn accepts_a_signed_manifest_for_the_selected_release() {
        let release = create_release(
            "v0.4.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sha256",
                "medotmd-aarch64-apple-darwin.tar.gz.sig",
                "medotmd-aarch64-apple-darwin.manifest",
                "medotmd-aarch64-apple-darwin.manifest.sig",
            ],
        );
        let current_version = Version::parse("0.3.0").expect("version should parse");
        let update_plan = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .expect("release should be valid")
            .expect("update should be available");
        let manifest_contents = create_manifest(
            "0.4.0",
            "v0.4.0",
            "aarch64-apple-darwin",
            "medotmd-aarch64-apple-darwin.tar.gz",
            &"f".repeat(64),
        );
        let (signature_contents, public_key_bytes) = get_signature(manifest_contents.as_bytes());

        let release_manifest = verify_release_manifest_with_public_key(
            manifest_contents.as_bytes(),
            &signature_contents,
            &update_plan,
            &public_key_bytes,
        )
        .expect("manifest should be valid");

        assert_eq!(release_manifest.release_tag, "v0.4.0");
    }

    #[test]
    fn rejects_a_signed_manifest_replayed_from_an_older_release() {
        let release = create_release(
            "v0.5.0",
            &[
                "medotmd-aarch64-apple-darwin.tar.gz",
                "medotmd-aarch64-apple-darwin.tar.gz.sha256",
                "medotmd-aarch64-apple-darwin.tar.gz.sig",
                "medotmd-aarch64-apple-darwin.manifest",
                "medotmd-aarch64-apple-darwin.manifest.sig",
            ],
        );
        let current_version = Version::parse("0.4.0").expect("version should parse");
        let update_plan = get_update_plan(&release, &current_version, "aarch64-apple-darwin")
            .expect("release should be valid")
            .expect("update should be available");
        let manifest_contents = create_manifest(
            "0.4.0",
            "v0.4.0",
            "aarch64-apple-darwin",
            "medotmd-aarch64-apple-darwin.tar.gz",
            &"f".repeat(64),
        );
        let (signature_contents, public_key_bytes) = get_signature(manifest_contents.as_bytes());

        let result = verify_release_manifest_with_public_key(
            manifest_contents.as_bytes(),
            &signature_contents,
            &update_plan,
            &public_key_bytes,
        );
        let error = result.err().expect("replayed manifest should be rejected");

        assert!(error.to_string().contains("version does not match"));
    }

    #[test]
    fn accepts_an_archive_signed_by_the_trusted_key() {
        let archive_contents = create_archive(b"new binary");
        let (signature_contents, public_key_bytes) = get_signature(&archive_contents);

        verify_release_signature_with_public_key(
            &archive_contents,
            &signature_contents,
            &public_key_bytes,
        )
        .expect("signature should be valid");
    }

    #[test]
    fn accepts_a_fixture_signed_by_the_release_key() {
        let archive_contents = b"medotmd release signature fixture\n";
        let signature_contents = [
            166, 225, 99, 86, 78, 115, 109, 17, 112, 134, 62, 79, 24, 217, 94, 83, 162, 84, 236,
            228, 207, 150, 117, 155, 6, 7, 99, 40, 37, 81, 212, 1, 231, 27, 48, 87, 251, 33, 147,
            126, 1, 131, 246, 180, 59, 130, 131, 178, 71, 49, 230, 14, 81, 222, 218, 176, 55, 1,
            183, 97, 204, 65, 7, 5,
        ];

        verify_archive_signature(archive_contents, &signature_contents)
            .expect("signature should match the release key");
    }

    #[test]
    fn rejects_a_tampered_signed_archive() {
        let archive_contents = create_archive(b"new binary");
        let (signature_contents, public_key_bytes) = get_signature(&archive_contents);
        let mut tampered_archive_contents = archive_contents;
        tampered_archive_contents.push(0);

        let error = verify_release_signature_with_public_key(
            &tampered_archive_contents,
            &signature_contents,
            &public_key_bytes,
        )
        .expect_err("signature should be rejected");

        assert!(error.to_string().contains("verification failed"));
    }

    #[test]
    fn rejects_an_archive_that_does_not_match_the_signed_manifest() {
        let archive_contents = create_archive(b"new binary");
        let release_manifest = super::ReleaseManifest {
            version: "0.4.0".to_owned(),
            release_tag: "v0.4.0".to_owned(),
            release_target: "aarch64-apple-darwin".to_owned(),
            archive_file_name: "medotmd-aarch64-apple-darwin.tar.gz".to_owned(),
            archive_checksum: "0".repeat(64),
        };

        let error = verify_manifest_archive_checksum(&archive_contents, &release_manifest)
            .expect_err("archive should be rejected");

        assert!(error.to_string().contains("signed manifest"));
    }

    #[test]
    fn configures_a_bounded_https_only_update_agent() {
        let update_agent = get_update_agent();
        let configuration = update_agent.config();
        let timeouts = configuration.timeouts();

        assert!(configuration.https_only());
        assert_eq!(configuration.max_redirects(), MAX_REDIRECTS);
        assert_eq!(
            configuration.max_response_header_size(),
            MAX_RESPONSE_HEADER_BYTES
        );
        assert_eq!(timeouts.global, Some(UPDATE_GLOBAL_TIMEOUT));
        assert_eq!(timeouts.per_call, Some(UPDATE_CALL_TIMEOUT));
        assert_eq!(timeouts.resolve, Some(UPDATE_RESOLVE_TIMEOUT));
        assert_eq!(timeouts.connect, Some(UPDATE_CONNECT_TIMEOUT));
        assert_eq!(timeouts.recv_response, Some(UPDATE_RESPONSE_TIMEOUT));
        assert_eq!(timeouts.recv_body, Some(UPDATE_BODY_TIMEOUT));
    }

    #[test]
    fn rejects_an_oversized_declared_response() {
        let error = validate_response_content_length(Some("1025"), 1_024, "release signature")
            .expect_err("oversized response should be rejected");

        assert!(error.to_string().contains("release signature is too large"));
    }

    #[test]
    fn rejects_an_oversized_medotmd_binary_before_extracting_it() {
        let archive_contents = create_archive(&[0; 128]);
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let extracted_binary_path = temporary_directory.path().join(BINARY_FILE_NAME);

        let error =
            extract_binary_with_limits(&archive_contents, &extracted_binary_path, 4_096, 64)
                .expect_err("oversized binary should be rejected");

        assert!(
            error
                .to_string()
                .contains("medotmd binary exceeds 64 bytes")
        );
        assert!(!extracted_binary_path.exists());
    }

    #[test]
    fn rejects_a_highly_compressible_archive_that_exceeds_the_decompressed_limit() {
        let padding_contents = vec![0; 8_192];
        let archive_contents = create_archive_with_entries(&[
            ("padding", &padding_contents),
            (BINARY_FILE_NAME, b"new binary"),
        ]);
        let temporary_directory = tempdir().expect("temporary directory should be created");
        let extracted_binary_path = temporary_directory.path().join(BINARY_FILE_NAME);

        assert!(archive_contents.len() < 1_024);

        let error =
            extract_binary_with_limits(&archive_contents, &extracted_binary_path, 1_024, 1_024)
                .expect_err("decompression bomb should be rejected");

        assert!(
            error
                .to_string()
                .contains("release archive decompressed size exceeds 1024 bytes")
        );
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
