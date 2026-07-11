use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use jsonc_parser::{ParseOptions, cst::CstRootNode};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new(test_name: &str) -> Self {
        let now_in_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "medotmd-{test_name}-{}-{now_in_nanos}",
            std::process::id()
        ));

        fs::create_dir_all(&path).expect("temp home should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_medotmd(home_path: &Path, args: &[&str]) -> Output {
    run_medotmd_with_environment(home_path, args, &[])
}

fn run_medotmd_with_environment(
    home_path: &Path,
    args: &[&str],
    environment_variables: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_medotmd"));
    command
        .args(args)
        .env("HOME", home_path)
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .envs(environment_variables.iter().copied());

    command.output().expect("medotmd should run")
}

fn assert_success(output: Output) -> String {
    if !output.status.success() {
        panic!(
            "command failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn assert_failure(output: Output) -> String {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).expect("file should be readable")
}

fn count_imports(content: &str, import_line: &str) -> usize {
    content.lines().filter(|line| *line == import_line).count()
}

fn count_occurrences(content: &str, value: &str) -> usize {
    content.match_indices(value).count()
}

fn get_instruction_paths(config_content: &str) -> Vec<String> {
    let config_root = CstRootNode::parse(config_content, &ParseOptions::default())
        .expect("OpenCode config should parse");
    let instructions = config_root
        .object_value()
        .and_then(|config_object| config_object.array_value("instructions"))
        .expect("OpenCode config should contain instructions");

    instructions
        .elements()
        .into_iter()
        .map(|instruction| {
            instruction
                .as_string_lit()
                .expect("instruction should be a string")
                .decoded_value()
                .expect("instruction string should decode")
        })
        .collect()
}

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
fn get_mode(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("path metadata should be readable")
        .permissions()
        .mode()
        & 0o7777
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .expect("path permissions should be set");
}

#[cfg(unix)]
fn create_editor_script(
    temp_home: &TempHome,
    file_name: &str,
    marker: &str,
    exit_code: u8,
) -> PathBuf {
    let editor_path = temp_home.path().join(file_name);
    let editor_script = format!(
        "#!/bin/sh\nprintf '%s\\n' '{marker}' \"$@\" > \"$MEDOTMD_TEST_EDITOR_OUTPUT\"\nexit {exit_code}\n"
    );
    fs::write(&editor_path, editor_script).expect("editor script should be written");
    set_mode(&editor_path, 0o700);

    editor_path
}

#[test]
fn init_creates_identity_file_and_installs_detected_agents() {
    let temp_home = TempHome::new("init");
    let codex_folder_path = temp_home.path().join(".codex");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let opencode_agents_file_path = opencode_folder_path.join("AGENTS.md");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(&opencode_folder_path).expect("opencode folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");
    fs::write(
        &opencode_agents_file_path,
        "existing OpenCode instructions\n",
    )
    .expect("OpenCode instructions should be written");

    let output = assert_success(run_medotmd(temp_home.path(), &["init"]));
    let identity_import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let guidance_import_line = format!("@{}/.me/AGENT.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);
    let opencode_config_content = read_to_string(&opencode_config_file_path);

    assert!(output.contains("• Initializing medotmd"));
    assert!(output.contains("✓ ME.md created"));
    assert!(output.contains("✓ AGENT.md created"));
    assert!(output.contains("✓ Codex installed"));
    assert!(output.contains("✓ OpenCode created"));
    assert!(output.contains("! Claude skipped, folder missing"));
    assert!(temp_home.path().join(".me/ME.md").exists());
    assert!(read_to_string(&temp_home.path().join(".me/AGENT.md")).contains("Profile maintenance"));
    assert!(
        read_to_string(&temp_home.path().join(".me/AGENT.md"))
            .contains("Do not inspect or evaluate ME.md")
    );
    assert_eq!(count_imports(&codex_content, &identity_import_line), 1);
    assert_eq!(count_imports(&codex_content, &guidance_import_line), 1);
    assert!(codex_content.contains("existing codex"));
    assert_eq!(
        count_occurrences(
            &opencode_config_content,
            &temp_home.path().join(".me/ME.md").display().to_string(),
        ),
        1
    );
    assert_eq!(
        count_occurrences(
            &opencode_config_content,
            &temp_home.path().join(".me/AGENT.md").display().to_string(),
        ),
        1
    );
    assert_eq!(
        read_to_string(&opencode_agents_file_path),
        "existing OpenCode instructions\n"
    );
    assert!(!temp_home.path().join(".claude/CLAUDE.md").exists());
}

#[cfg(unix)]
#[test]
fn init_creates_private_profile_directory_and_files() {
    let temp_home = TempHome::new("private-profile-create");

    assert_success(run_medotmd(temp_home.path(), &["init", "--agent", "codex"]));

    assert_eq!(get_mode(&temp_home.path().join(".me")), 0o700);
    assert_eq!(get_mode(&temp_home.path().join(".me/ME.md")), 0o600);
    assert_eq!(get_mode(&temp_home.path().join(".me/AGENT.md")), 0o600);
}

#[cfg(unix)]
#[test]
fn install_repairs_insecure_profile_permissions_without_changing_contents() {
    let temp_home = TempHome::new("private-profile-repair");
    let profile_directory_path = temp_home.path().join(".me");
    let identity_file_path = profile_directory_path.join("ME.md");
    let guidance_file_path = profile_directory_path.join("AGENT.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::write(&identity_file_path, "# Private profile\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "Private guidance\n").expect("guidance file should be written");
    set_mode(&profile_directory_path, 0o755);
    set_mode(&identity_file_path, 0o644);
    set_mode(&guidance_file_path, 0o604);

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "codex"],
    ));

    assert!(output.contains("✓ .me directory permissions secured"));
    assert!(output.contains("✓ ME.md permissions secured"));
    assert!(output.contains("✓ AGENT.md permissions secured"));
    assert_eq!(read_to_string(&identity_file_path), "# Private profile\n");
    assert_eq!(read_to_string(&guidance_file_path), "Private guidance\n");
    assert_eq!(get_mode(&profile_directory_path), 0o700);
    assert_eq!(get_mode(&identity_file_path), 0o600);
    assert_eq!(get_mode(&guidance_file_path), 0o600);
}

#[cfg(unix)]
#[test]
fn install_dry_run_reports_insecure_profile_permission_repairs_without_changing_them() {
    let temp_home = TempHome::new("private-profile-dry-run");
    let profile_directory_path = temp_home.path().join(".me");
    let identity_file_path = profile_directory_path.join("ME.md");
    let guidance_file_path = profile_directory_path.join("AGENT.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::write(&identity_file_path, "# Private profile\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "Private guidance\n").expect("guidance file should be written");
    set_mode(&profile_directory_path, 0o755);
    set_mode(&identity_file_path, 0o644);
    set_mode(&guidance_file_path, 0o604);

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--dry-run", "--agent", "codex"],
    ));

    assert!(output.contains("! .me directory permissions would be secured"));
    assert!(output.contains("! ME.md permissions would be secured"));
    assert!(output.contains("! AGENT.md permissions would be secured"));
    assert_eq!(read_to_string(&identity_file_path), "# Private profile\n");
    assert_eq!(read_to_string(&guidance_file_path), "Private guidance\n");
    assert_eq!(get_mode(&profile_directory_path), 0o755);
    assert_eq!(get_mode(&identity_file_path), 0o644);
    assert_eq!(get_mode(&guidance_file_path), 0o604);
}

#[cfg(unix)]
#[test]
fn doctor_reports_insecure_profile_permissions() {
    let temp_home = TempHome::new("private-profile-doctor");
    let profile_directory_path = temp_home.path().join(".me");
    let identity_file_path = profile_directory_path.join("ME.md");
    let guidance_file_path = profile_directory_path.join("AGENT.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::write(&identity_file_path, "# Private profile\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "Private guidance\n").expect("guidance file should be written");
    set_mode(&profile_directory_path, 0o755);
    set_mode(&identity_file_path, 0o644);
    set_mode(&guidance_file_path, 0o604);

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✗ .me directory permissions insecure (expected 0700, found 0755)"));
    assert!(output.contains("✗ ME.md permissions insecure (expected 0600, found 0644)"));
    assert!(output.contains("✗ AGENT.md permissions insecure (expected 0600, found 0604)"));
}

#[cfg(unix)]
#[test]
fn install_rejects_symlinked_profile_directory() {
    let temp_home = TempHome::new("symlinked-profile-directory");
    let outside_directory_path = temp_home.path().join("outside-profile");
    let outside_identity_file_path = outside_directory_path.join("ME.md");

    fs::create_dir(&outside_directory_path).expect("outside directory should be created");
    fs::write(&outside_identity_file_path, "outside profile\n")
        .expect("outside identity file should be written");
    set_mode(&outside_directory_path, 0o755);
    set_mode(&outside_identity_file_path, 0o644);
    symlink(&outside_directory_path, temp_home.path().join(".me"))
        .expect("profile directory symlink should be created");

    let output = run_medotmd(temp_home.path(), &["install", "--agent", "codex"]);

    assert!(!output.status.success());
    assert_eq!(
        read_to_string(&outside_identity_file_path),
        "outside profile\n"
    );
    assert_eq!(get_mode(&outside_directory_path), 0o755);
    assert_eq!(get_mode(&outside_identity_file_path), 0o644);
}

#[cfg(unix)]
#[test]
fn install_rejects_symlinked_profile_file() {
    let temp_home = TempHome::new("symlinked-profile-file");
    let profile_directory_path = temp_home.path().join(".me");
    let outside_identity_file_path = temp_home.path().join("outside-profile.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::write(&outside_identity_file_path, "outside profile\n")
        .expect("outside identity file should be written");
    set_mode(&profile_directory_path, 0o700);
    set_mode(&outside_identity_file_path, 0o644);
    symlink(
        &outside_identity_file_path,
        profile_directory_path.join("ME.md"),
    )
    .expect("identity file symlink should be created");

    let output = run_medotmd(temp_home.path(), &["install", "--agent", "codex"]);

    assert!(!output.status.success());
    assert_eq!(
        read_to_string(&outside_identity_file_path),
        "outside profile\n"
    );
    assert_eq!(get_mode(&outside_identity_file_path), 0o644);
    assert!(!profile_directory_path.join("AGENT.md").exists());
}

#[cfg(unix)]
#[test]
fn doctor_does_not_follow_a_symlinked_profile_directory() {
    let temp_home = TempHome::new("doctor-symlinked-profile-directory");
    let outside_directory_path = temp_home.path().join("outside-profile");
    let outside_identity_file_path = outside_directory_path.join("ME.md");
    let outside_guidance_file_path = outside_directory_path.join("AGENT.md");

    fs::create_dir(&outside_directory_path).expect("outside directory should be created");
    fs::write(&outside_identity_file_path, "outside profile\n")
        .expect("outside identity file should be written");
    fs::write(&outside_guidance_file_path, "outside guidance\n")
        .expect("outside guidance file should be written");
    set_mode(&outside_directory_path, 0o700);
    set_mode(&outside_identity_file_path, 0o600);
    set_mode(&outside_guidance_file_path, 0o600);
    symlink(&outside_directory_path, temp_home.path().join(".me"))
        .expect("profile directory symlink should be created");

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✗ .me directory is a symbolic link"));
    assert!(!output.contains("ME.md exists"));
    assert!(!output.contains("AGENT.md exists"));
}

#[cfg(unix)]
#[test]
fn print_rejects_a_symlinked_identity_file() {
    let temp_home = TempHome::new("print-symlinked-identity-file");
    let profile_directory_path = temp_home.path().join(".me");
    let outside_identity_file_path = temp_home.path().join("outside-profile.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::write(&outside_identity_file_path, "outside profile\n")
        .expect("outside identity file should be written");
    set_mode(&profile_directory_path, 0o700);
    set_mode(&outside_identity_file_path, 0o600);
    symlink(
        &outside_identity_file_path,
        profile_directory_path.join("ME.md"),
    )
    .expect("identity file symlink should be created");

    let output = run_medotmd(temp_home.path(), &["print"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not be a symbolic link"));
    assert!(output.stdout.is_empty());
    assert_eq!(
        read_to_string(&outside_identity_file_path),
        "outside profile\n"
    );
}

#[cfg(unix)]
#[test]
fn print_rejects_a_non_regular_identity_path() {
    let temp_home = TempHome::new("print-non-regular-identity-file");
    let profile_directory_path = temp_home.path().join(".me");
    let identity_file_path = profile_directory_path.join("ME.md");

    fs::create_dir(&profile_directory_path).expect("profile directory should be created");
    fs::create_dir(&identity_file_path).expect("identity directory should be created");
    set_mode(&profile_directory_path, 0o700);
    set_mode(&identity_file_path, 0o700);

    let output = run_medotmd(temp_home.path(), &["print"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be a regular file"));
    assert!(output.stdout.is_empty());
}

#[test]
fn opencode_install_and_uninstall_preserve_existing_configuration() {
    let temp_home = TempHome::new("opencode-existing-config");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let custom_instruction_path = temp_home.path().join("custom-instructions.md");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&custom_instruction_path, "custom instructions\n")
        .expect("custom instructions should be written");
    fs::write(
        &opencode_config_file_path,
        format!(
            "{{\n  // preserve this comment\n  \"model\": \"openai/gpt-5\",\n  \"instructions\": [\n    \"{}\"\n  ]\n}}\n",
            custom_instruction_path.display()
        ),
    )
    .expect("OpenCode config should be written");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    let installed_config_content = read_to_string(&opencode_config_file_path);
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");

    assert!(installed_config_content.contains("// preserve this comment"));
    assert!(installed_config_content.contains("\"model\": \"openai/gpt-5\""));
    assert_eq!(
        count_occurrences(
            &installed_config_content,
            &custom_instruction_path.display().to_string()
        ),
        1
    );
    assert_eq!(
        count_occurrences(
            &installed_config_content,
            &identity_file_path.display().to_string()
        ),
        1
    );
    assert_eq!(
        count_occurrences(
            &installed_config_content,
            &guidance_file_path.display().to_string()
        ),
        1
    );

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 1);

    assert_success(run_medotmd(
        temp_home.path(),
        &["uninstall", "--agent", "opencode"],
    ));

    let uninstalled_config_content = read_to_string(&opencode_config_file_path);

    assert!(uninstalled_config_content.contains("// preserve this comment"));
    assert!(uninstalled_config_content.contains("\"model\": \"openai/gpt-5\""));
    assert_eq!(
        count_occurrences(
            &uninstalled_config_content,
            &custom_instruction_path.display().to_string()
        ),
        1
    );
    assert_eq!(
        count_occurrences(
            &uninstalled_config_content,
            &identity_file_path.display().to_string()
        ),
        0
    );
    assert_eq!(
        count_occurrences(
            &uninstalled_config_content,
            &guidance_file_path.display().to_string()
        ),
        0
    );
}

#[test]
fn opencode_install_and_uninstall_only_modify_the_effective_configuration() {
    let temp_home = TempHome::new("opencode-effective-configuration");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let fallback_config_file_path = opencode_folder_path.join("config.json");
    let effective_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let fallback_instruction_path = temp_home.path().join("fallback-instructions.md");
    let effective_instruction_path = temp_home.path().join("effective-instructions.md");
    let fallback_config_content = format!(
        "{{\n  \"instructions\": [\n    \"{}\",\n    \"{}\",\n    \"{}\"\n  ]\n}}\n",
        fallback_instruction_path.display(),
        guidance_file_path.display(),
        identity_file_path.display()
    );

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&fallback_config_file_path, &fallback_config_content)
        .expect("fallback OpenCode config should be written");
    fs::write(
        &effective_config_file_path,
        format!(
            "{{\n  \"instructions\": [\n    \"{}\"\n  ]\n}}\n",
            effective_instruction_path.display()
        ),
    )
    .expect("effective OpenCode config should be written");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    assert_eq!(
        read_to_string(&fallback_config_file_path),
        fallback_config_content
    );
    assert_eq!(backup_count(&opencode_folder_path, "config.json"), 0);
    assert_eq!(
        get_instruction_paths(&read_to_string(&effective_config_file_path)),
        vec![
            effective_instruction_path.display().to_string(),
            guidance_file_path.display().to_string(),
            identity_file_path.display().to_string(),
        ]
    );
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 1);

    assert_success(run_medotmd(
        temp_home.path(),
        &["uninstall", "--agent", "opencode"],
    ));

    assert_eq!(
        read_to_string(&fallback_config_file_path),
        fallback_config_content
    );
    assert_eq!(
        get_instruction_paths(&read_to_string(&effective_config_file_path)),
        vec![effective_instruction_path.display().to_string()]
    );
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 2);
}

#[test]
fn opencode_install_dry_run_does_not_change_existing_configuration() {
    let temp_home = TempHome::new("opencode-dry-run");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let opencode_agents_file_path = opencode_folder_path.join("AGENTS.md");
    let original_config_content = "{\n  \"model\": \"openai/gpt-5\"\n}\n";
    let original_agents_content = format!(
        "existing OpenCode instructions\n@{}/.me/AGENT.md\n@{}/.me/ME.md\n",
        temp_home.path().display(),
        temp_home.path().display()
    );

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&opencode_config_file_path, original_config_content)
        .expect("OpenCode config should be written");
    fs::write(&opencode_agents_file_path, &original_agents_content)
        .expect("OpenCode instructions should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--dry-run", "--agent", "opencode"],
    ));

    assert!(output.contains("! OpenCode would be installed"));
    assert_eq!(
        read_to_string(&opencode_config_file_path),
        original_config_content
    );
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 0);
    assert_eq!(
        read_to_string(&opencode_agents_file_path),
        original_agents_content
    );
    assert_eq!(backup_count(&opencode_folder_path, "AGENTS.md"), 0);
    assert!(!temp_home.path().join(".me/ME.md").exists());
    assert!(!temp_home.path().join(".me/AGENT.md").exists());
}

#[test]
fn opencode_install_migrates_legacy_imports_without_touching_other_content() {
    let temp_home = TempHome::new("opencode-legacy-install");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let opencode_agents_file_path = opencode_folder_path.join("AGENTS.md");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let original_agents_content = format!(
        "before\n@{}\nuser-owned instructions\n@{}\nafter\n",
        guidance_file_path.display(),
        identity_file_path.display()
    );

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(
        &opencode_config_file_path,
        format!(
            "{{\n  \"instructions\": [\n    \"{}\",\n    \"{}\"\n  ]\n}}\n",
            guidance_file_path.display(),
            identity_file_path.display()
        ),
    )
    .expect("OpenCode config should be written");
    fs::write(&opencode_agents_file_path, original_agents_content)
        .expect("OpenCode instructions should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    assert!(output.contains("✓ OpenCode installed"));
    assert_eq!(
        read_to_string(&opencode_agents_file_path),
        "before\nuser-owned instructions\nafter\n"
    );
    assert_eq!(backup_count(&opencode_folder_path, "AGENTS.md"), 1);
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 0);
}

#[test]
fn opencode_uninstall_removes_legacy_imports_without_a_configuration() {
    let temp_home = TempHome::new("opencode-legacy-uninstall");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_agents_file_path = opencode_folder_path.join("AGENTS.md");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(
        &opencode_agents_file_path,
        format!(
            "before\n@{}\nuser-owned instructions\n@{}\nafter\n",
            guidance_file_path.display(),
            identity_file_path.display()
        ),
    )
    .expect("OpenCode instructions should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["uninstall", "--agent", "opencode"],
    ));

    assert!(output.contains("✓ OpenCode uninstalled"));
    assert_eq!(
        read_to_string(&opencode_agents_file_path),
        "before\nuser-owned instructions\nafter\n"
    );
    assert_eq!(backup_count(&opencode_folder_path, "AGENTS.md"), 1);
}

#[test]
fn opencode_install_escapes_json_special_characters_in_profile_paths() {
    let temp_home = TempHome::new("opencode-json-special-\"\\");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    let config_content = read_to_string(&opencode_config_file_path);

    assert!(config_content.contains("\\\""));
    assert!(config_content.contains("\\\\"));
    assert_eq!(
        get_instruction_paths(&config_content),
        vec![
            guidance_file_path.display().to_string(),
            identity_file_path.display().to_string(),
        ]
    );
}

#[test]
fn opencode_install_preserves_existing_profile_instruction_order() {
    let temp_home = TempHome::new("opencode-existing-profile-order");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let custom_instruction_path = temp_home.path().join("custom-instructions.md");
    let original_config_content = format!(
        "{{\n  \"instructions\": [\n    \"{}\",\n    \"{}\",\n    \"{}\"\n  ]\n}}\n",
        identity_file_path.display(),
        custom_instruction_path.display(),
        guidance_file_path.display()
    );

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&opencode_config_file_path, &original_config_content)
        .expect("OpenCode config should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "opencode"],
    ));

    assert!(output.contains("✓ OpenCode already installed"));
    assert_eq!(
        read_to_string(&opencode_config_file_path),
        original_config_content
    );
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 0);
}

#[test]
fn opencode_uninstall_preserves_an_empty_configuration() {
    let temp_home = TempHome::new("opencode-empty-config");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&opencode_config_file_path, "").expect("OpenCode config should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["uninstall", "--agent", "opencode"],
    ));

    assert!(output.contains("✓ OpenCode not installed"));
    assert_eq!(read_to_string(&opencode_config_file_path), "");
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 0);
}

#[test]
fn doctor_reports_missing_opencode_profile_instruction() {
    let temp_home = TempHome::new("opencode-doctor");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::create_dir_all(
        identity_file_path
            .parent()
            .expect("identity folder should exist"),
    )
    .expect("identity folder should be created");
    fs::write(&identity_file_path, "# Me\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "guidance\n").expect("guidance file should be written");
    fs::write(
        &opencode_config_file_path,
        format!(
            "{{\n  \"instructions\": [\n    \"{}\"\n  ]\n}}\n",
            guidance_file_path.display()
        ),
    )
    .expect("OpenCode config should be written");

    #[cfg(unix)]
    {
        set_mode(
            identity_file_path
                .parent()
                .expect("identity folder should exist"),
            0o700,
        );
        set_mode(&identity_file_path, 0o600);
        set_mode(&guidance_file_path, 0o600);
    }

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "opencode"],
    ));

    assert!(output.contains("✗ OpenCode missing ME.md instruction"));
}

#[test]
fn doctor_uses_the_highest_precedence_opencode_configuration() {
    let temp_home = TempHome::new("opencode-doctor-precedence");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let fallback_config_file_path = opencode_folder_path.join("config.json");
    let effective_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let unrelated_instruction_path = temp_home.path().join("unrelated-instructions.md");

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::create_dir_all(
        identity_file_path
            .parent()
            .expect("identity folder should exist"),
    )
    .expect("identity folder should be created");
    fs::write(&identity_file_path, "# Me\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "guidance\n").expect("guidance file should be written");
    fs::write(
        &fallback_config_file_path,
        format!(
            "{{\n  \"instructions\": [\n    \"{}\",\n    \"{}\"\n  ]\n}}\n",
            guidance_file_path.display(),
            identity_file_path.display()
        ),
    )
    .expect("fallback OpenCode config should be written");
    fs::write(
        &effective_config_file_path,
        format!(
            "{{\n  \"instructions\": [\n    \"{}\"\n  ]\n}}\n",
            unrelated_instruction_path.display()
        ),
    )
    .expect("effective OpenCode config should be written");

    #[cfg(unix)]
    {
        set_mode(
            identity_file_path
                .parent()
                .expect("identity folder should exist"),
            0o700,
        );
        set_mode(&identity_file_path, 0o600);
        set_mode(&guidance_file_path, 0o600);
    }

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "opencode"],
    ));

    assert!(output.contains("✗ OpenCode missing AGENT.md instruction"));
    assert!(!output.contains("✓ OpenCode installed"));
}

#[test]
fn opencode_install_rejects_malformed_configuration_without_overwriting_it() {
    let temp_home = TempHome::new("opencode-invalid-config");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let opencode_config_file_path = opencode_folder_path.join("opencode.jsonc");
    let original_config_content = "{\n  \"instructions\": [true]\n}\n";

    fs::create_dir_all(&opencode_folder_path).expect("OpenCode folder should be created");
    fs::write(&opencode_config_file_path, original_config_content)
        .expect("OpenCode config should be written");

    let output = run_medotmd(temp_home.path(), &["install", "--agent", "opencode"]);

    assert!(!output.status.success());
    assert_eq!(
        read_to_string(&opencode_config_file_path),
        original_config_content
    );
    assert_eq!(backup_count(&opencode_folder_path, "opencode.jsonc"), 0);
}

#[test]
fn install_dry_run_does_not_change_files() {
    let temp_home = TempHome::new("dry-run");
    let claude_folder_path = temp_home.path().join(".claude");
    let claude_file_path = claude_folder_path.join("CLAUDE.md");

    fs::create_dir_all(&claude_folder_path).expect("claude folder should be created");
    fs::write(&claude_file_path, "existing claude\n").expect("claude file should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--dry-run", "--agent", "claude"],
    ));

    assert!(output.contains("would be created"));
    assert!(output.contains("would be installed"));
    assert!(output.contains("! ME.md would be created"));
    assert!(output.contains("! AGENT.md would be created"));
    assert!(output.contains("! Claude would be installed"));
    assert!(!temp_home.path().join(".me/ME.md").exists());
    assert!(!temp_home.path().join(".me/AGENT.md").exists());
    assert_eq!(read_to_string(&claude_file_path), "existing claude\n");
    assert_eq!(backup_count(&claude_folder_path, "CLAUDE.md"), 0);
}

#[test]
fn install_agent_filter_only_installs_selected_agent() {
    let temp_home = TempHome::new("agent-filter");
    let codex_folder_path = temp_home.path().join(".codex");
    let claude_folder_path = temp_home.path().join(".claude");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let claude_file_path = claude_folder_path.join("CLAUDE.md");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(&claude_folder_path).expect("claude folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");
    fs::write(&claude_file_path, "existing claude\n").expect("claude file should be written");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "claude"],
    ));

    let identity_import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let guidance_import_line = format!("@{}/.me/AGENT.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);
    let claude_content = read_to_string(&claude_file_path);

    assert_eq!(count_imports(&codex_content, &identity_import_line), 0);
    assert_eq!(count_imports(&codex_content, &guidance_import_line), 0);
    assert_eq!(count_imports(&claude_content, &identity_import_line), 1);
    assert_eq!(count_imports(&claude_content, &guidance_import_line), 1);
}

#[test]
fn install_upgrades_a_me_only_target() {
    let temp_home = TempHome::new("upgrade");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let identity_import_line = format!("@{}", identity_file_path.display());
    let guidance_import_line = format!("@{}/.me/AGENT.md", temp_home.path().display());

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(
        identity_file_path
            .parent()
            .expect("identity folder should exist"),
    )
    .expect("identity folder should be created");
    fs::write(&identity_file_path, "# Me\n").expect("identity file should be written");
    fs::write(
        &codex_file_path,
        format!("{identity_import_line}\nexisting codex\n"),
    )
    .expect("codex file should be written");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "codex"],
    ));

    let codex_content = read_to_string(&codex_file_path);

    assert_eq!(count_imports(&codex_content, &identity_import_line), 1);
    assert_eq!(count_imports(&codex_content, &guidance_import_line), 1);
    assert!(codex_content.ends_with("existing codex\n"));
    assert_eq!(backup_count(&codex_folder_path, "AGENTS.md"), 1);
}

#[test]
fn doctor_reports_missing_guidance_import() {
    let temp_home = TempHome::new("doctor-imports");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let identity_import_line = format!("@{}", identity_file_path.display());

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(
        identity_file_path
            .parent()
            .expect("identity folder should exist"),
    )
    .expect("identity folder should be created");
    fs::write(&identity_file_path, "# Me\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "guidance\n").expect("guidance file should be written");
    fs::write(&codex_file_path, format!("{identity_import_line}\n"))
        .expect("codex file should be written");

    #[cfg(unix)]
    {
        set_mode(
            identity_file_path
                .parent()
                .expect("identity folder should exist"),
            0o700,
        );
        set_mode(&identity_file_path, 0o600);
        set_mode(&guidance_file_path, 0o600);
    }

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✓ .me directory permissions private"));
    assert!(output.contains("✓ ME.md exists"));
    assert!(output.contains("✓ AGENT.md exists"));
    assert!(output.contains("✗ Codex missing AGENT.md import"));
}

#[cfg(unix)]
#[test]
fn doctor_rejects_a_symlinked_agent_instruction_file() {
    let temp_home = TempHome::new("doctor-symlinked-agent-file");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let external_file_path = temp_home.path().join("external-instructions.md");

    fs::create_dir_all(&codex_folder_path).expect("Codex folder should be created");
    assert_success(run_medotmd(temp_home.path(), &["init", "--agent", "codex"]));
    fs::write(&external_file_path, read_to_string(&codex_file_path))
        .expect("external instruction file should be written");
    fs::remove_file(&codex_file_path).expect("Codex instruction file should be removed");
    symlink(&external_file_path, &codex_file_path)
        .expect("Codex instruction file should be linked");

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✗ Codex target file is a symbolic link"));
    assert!(!output.contains("✓ Codex installed"));
}

#[test]
fn doctor_reports_duplicated_guidance_import() {
    let temp_home = TempHome::new("doctor-duplicates");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let identity_file_path = temp_home.path().join(".me/ME.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");
    let identity_import_line = format!("@{}", identity_file_path.display());
    let guidance_import_line = format!("@{}", guidance_file_path.display());

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(
        identity_file_path
            .parent()
            .expect("identity folder should exist"),
    )
    .expect("identity folder should be created");
    fs::write(&identity_file_path, "# Me\n").expect("identity file should be written");
    fs::write(&guidance_file_path, "guidance\n").expect("guidance file should be written");
    fs::write(
        &codex_file_path,
        format!("{guidance_import_line}\n{guidance_import_line}\n{identity_import_line}\n"),
    )
    .expect("codex file should be written");

    let output = assert_failure(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✗ Codex duplicated AGENT.md import (2)"));
}

#[test]
fn doctor_warnings_are_nonfatal() {
    let temp_home = TempHome::new("doctor-guidance");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let guidance_file_path = temp_home.path().join(".me/AGENT.md");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");
    assert_success(run_medotmd(temp_home.path(), &["init", "--agent", "codex"]));
    fs::write(&guidance_file_path, "").expect("guidance file should be written");

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("! AGENT.md exists but is empty"));
    assert!(output.contains("✓ Codex installed"));
}

#[test]
fn doctor_returns_success_for_a_healthy_setup() {
    let temp_home = TempHome::new("doctor-healthy");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");
    assert_success(run_medotmd(temp_home.path(), &["init", "--agent", "codex"]));

    let output = assert_success(run_medotmd(
        temp_home.path(),
        &["doctor", "--agent", "codex"],
    ));

    assert!(output.contains("✓ .me directory permissions private"));
    assert!(output.contains("✓ ME.md exists"));
    assert!(output.contains("✓ AGENT.md exists"));
    assert!(output.contains("✓ Codex installed"));
}

#[test]
fn uninstall_removes_exact_imports_and_preserves_profile_files() {
    let temp_home = TempHome::new("uninstall");
    let codex_folder_path = temp_home.path().join(".codex");
    let codex_file_path = codex_folder_path.join("AGENTS.md");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");

    assert_success(run_medotmd(
        temp_home.path(),
        &["install", "--agent", "codex"],
    ));
    assert_success(run_medotmd(
        temp_home.path(),
        &["uninstall", "--agent", "codex"],
    ));

    let identity_import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let guidance_import_line = format!("@{}/.me/AGENT.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);

    assert_eq!(count_imports(&codex_content, &identity_import_line), 0);
    assert_eq!(count_imports(&codex_content, &guidance_import_line), 0);
    assert_eq!(codex_content, "existing codex\n");
    assert_eq!(backup_count(&codex_folder_path, "AGENTS.md"), 2);
    assert!(temp_home.path().join(".me/ME.md").exists());
    assert!(temp_home.path().join(".me/AGENT.md").exists());
}

#[cfg(unix)]
#[test]
fn edit_passes_configured_editor_arguments_without_shell_execution() {
    let temp_home = TempHome::new("edit-arguments");
    let editor_path = create_editor_script(&temp_home, "editor", "editor", 0);
    let editor_output_path = temp_home.path().join("editor-output");
    let unexpected_file_path = temp_home.path().join("unexpected-file");
    let editor_command = format!(
        "'{}' --wait \"line two\" ; touch {}",
        editor_path.display(),
        unexpected_file_path.display()
    );

    assert_success(run_medotmd_with_environment(
        temp_home.path(),
        &["edit"],
        &[
            ("EDITOR", &editor_command),
            (
                "MEDOTMD_TEST_EDITOR_OUTPUT",
                editor_output_path
                    .to_str()
                    .expect("editor output path should be utf8"),
            ),
        ],
    ));

    assert_eq!(
        read_to_string(&editor_output_path),
        format!(
            "editor\n--wait\nline two\n;\ntouch\n{}\n{}\n",
            unexpected_file_path.display(),
            temp_home.path().join(".me/ME.md").display()
        )
    );
    assert!(!unexpected_file_path.exists());
}

#[cfg(unix)]
#[test]
fn edit_supports_quoted_editor_paths() {
    let temp_home = TempHome::new("edit-quoted-path");
    let editor_path = create_editor_script(&temp_home, "editor with spaces", "editor", 0);
    let editor_output_path = temp_home.path().join("editor-output");
    let editor_command = format!("\"{}\" --flag", editor_path.display());

    assert_success(run_medotmd_with_environment(
        temp_home.path(),
        &["edit"],
        &[
            ("EDITOR", &editor_command),
            (
                "MEDOTMD_TEST_EDITOR_OUTPUT",
                editor_output_path
                    .to_str()
                    .expect("editor output path should be utf8"),
            ),
        ],
    ));

    assert_eq!(
        read_to_string(&editor_output_path),
        format!(
            "editor\n--flag\n{}\n",
            temp_home.path().join(".me/ME.md").display()
        )
    );
}

#[cfg(unix)]
#[test]
fn edit_prefers_visual_over_editor() {
    let temp_home = TempHome::new("edit-visual-precedence");
    let visual_path = create_editor_script(&temp_home, "visual-editor", "visual", 0);
    let editor_path = create_editor_script(&temp_home, "editor", "editor", 0);
    let editor_output_path = temp_home.path().join("editor-output");
    let visual_command = format!("'{}' --visual", visual_path.display());
    let editor_command = format!("'{}' --editor", editor_path.display());

    assert_success(run_medotmd_with_environment(
        temp_home.path(),
        &["edit"],
        &[
            ("VISUAL", &visual_command),
            ("EDITOR", &editor_command),
            (
                "MEDOTMD_TEST_EDITOR_OUTPUT",
                editor_output_path
                    .to_str()
                    .expect("editor output path should be utf8"),
            ),
        ],
    ));

    assert_eq!(
        read_to_string(&editor_output_path),
        format!(
            "visual\n--visual\n{}\n",
            temp_home.path().join(".me/ME.md").display()
        )
    );
}

#[cfg(unix)]
#[test]
fn edit_rejects_malformed_or_empty_editor_commands_before_creating_the_profile() {
    let temp_home = TempHome::new("edit-invalid-command");
    let malformed_output = run_medotmd_with_environment(
        temp_home.path(),
        &["edit"],
        &[("EDITOR", "\"missing closing quote")],
    );

    assert!(!malformed_output.status.success());
    assert!(
        String::from_utf8_lossy(&malformed_output.stderr)
            .contains("EDITOR is not a valid editor command: missing closing quote")
    );
    assert!(!temp_home.path().join(".me/ME.md").exists());

    let empty_output =
        run_medotmd_with_environment(temp_home.path(), &["edit"], &[("EDITOR", "   ")]);

    assert!(!empty_output.status.success());
    assert!(
        String::from_utf8_lossy(&empty_output.stderr)
            .contains("EDITOR must contain an editor executable")
    );
    assert!(!temp_home.path().join(".me/ME.md").exists());
}

#[cfg(unix)]
#[test]
fn edit_reports_nonzero_editor_exit() {
    let temp_home = TempHome::new("edit-nonzero-exit");
    let editor_path = create_editor_script(&temp_home, "editor", "editor", 23);
    let editor_output_path = temp_home.path().join("editor-output");
    let editor_command = format!("'{}'", editor_path.display());
    let output = run_medotmd_with_environment(
        temp_home.path(),
        &["edit"],
        &[
            ("EDITOR", &editor_command),
            (
                "MEDOTMD_TEST_EDITOR_OUTPUT",
                editor_output_path
                    .to_str()
                    .expect("editor output path should be utf8"),
            ),
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("editor exited with exit status: 23"));
    assert_eq!(
        read_to_string(&editor_output_path),
        format!("editor\n{}\n", temp_home.path().join(".me/ME.md").display())
    );
}
