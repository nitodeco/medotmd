use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

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
    Command::new(env!("CARGO_BIN_EXE_medotmd"))
        .args(args)
        .env("HOME", home_path)
        .output()
        .expect("medotmd should run")
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

fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).expect("file should be readable")
}

fn count_imports(content: &str, import_line: &str) -> usize {
    content.lines().filter(|line| *line == import_line).count()
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

#[test]
fn init_creates_identity_file_and_installs_detected_agents() {
    let temp_home = TempHome::new("init");
    let codex_folder_path = temp_home.path().join(".codex");
    let opencode_folder_path = temp_home.path().join(".config/opencode");
    let codex_file_path = codex_folder_path.join("AGENTS.md");
    let opencode_file_path = opencode_folder_path.join("AGENTS.md");

    fs::create_dir_all(&codex_folder_path).expect("codex folder should be created");
    fs::create_dir_all(&opencode_folder_path).expect("opencode folder should be created");
    fs::write(&codex_file_path, "existing codex\n").expect("codex file should be written");

    let output = assert_success(run_medotmd(temp_home.path(), &["init"]));
    let import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);
    let opencode_content = read_to_string(&opencode_file_path);

    assert!(output.contains("Initializing medotmd"));
    assert!(temp_home.path().join(".me/ME.md").exists());
    assert_eq!(count_imports(&codex_content, &import_line), 1);
    assert!(codex_content.contains("existing codex"));
    assert_eq!(count_imports(&opencode_content, &import_line), 1);
    assert!(!temp_home.path().join(".claude/CLAUDE.md").exists());
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

    assert!(output.contains("would create"));
    assert!(output.contains("would install"));
    assert!(!temp_home.path().join(".me/ME.md").exists());
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

    let import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);
    let claude_content = read_to_string(&claude_file_path);

    assert_eq!(count_imports(&codex_content, &import_line), 0);
    assert_eq!(count_imports(&claude_content, &import_line), 1);
}

#[test]
fn uninstall_removes_exact_import_and_preserves_content() {
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

    let import_line = format!("@{}/.me/ME.md", temp_home.path().display());
    let codex_content = read_to_string(&codex_file_path);

    assert_eq!(count_imports(&codex_content, &import_line), 0);
    assert_eq!(codex_content, "existing codex\n");
    assert_eq!(backup_count(&codex_folder_path, "AGENTS.md"), 2);
    assert!(temp_home.path().join(".me/ME.md").exists());
}
