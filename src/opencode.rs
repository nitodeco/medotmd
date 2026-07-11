use std::{
    error::Error,
    path::{Path, PathBuf},
};

use jsonc_parser::{
    ParseOptions,
    cst::{CstNode, CstObject, CstRootNode},
};

use crate::{
    agent::get_opencode_config_file_path,
    error::CliResult,
    file_write::{FileContentSnapshot, read_file_content_snapshot, replace_file_if_unchanged},
    target::{InstallAction, UninstallAction},
};

const INSTRUCTIONS_PROPERTY_NAME: &str = "instructions";

pub enum OpenCodeStatus {
    Installed,
    InstructionMissing(&'static str),
    Duplicated(&'static str, usize),
    ConfigurationMissing,
    Invalid(String),
}

struct OpenCodeConfig {
    path: PathBuf,
    original_content_snapshot: FileContentSnapshot,
    content: String,
}

enum LegacyAgentsAction {
    Modified,
    Unchanged,
    WouldModify,
}

pub fn install_opencode(
    opencode_folder_path: &Path,
    identity_file_path: &Path,
    guidance_file_path: &Path,
    is_dry_run: bool,
) -> CliResult<InstallAction> {
    let profile_paths = get_profile_paths(identity_file_path, guidance_file_path);
    let config_file_path = get_opencode_config_file_path(opencode_folder_path);
    let maybe_config = read_opencode_config(&config_file_path)?;
    let does_create_config = maybe_config.is_none();
    let mut config = maybe_config.unwrap_or_else(|| OpenCodeConfig {
        path: config_file_path,
        original_content_snapshot: FileContentSnapshot::missing(),
        content: String::new(),
    });
    config.content = normalize_profile_paths(&config.path, &config.content, &profile_paths)?;
    let is_modified = config.original_content_snapshot.content() != Some(config.content.as_str());

    let config_action = if !is_modified {
        InstallAction::Unchanged
    } else if is_dry_run {
        if does_create_config {
            InstallAction::WouldCreate
        } else {
            InstallAction::WouldModify
        }
    } else {
        replace_file_if_unchanged(
            &config.path,
            &config.original_content_snapshot,
            &config.content,
        )?;

        if does_create_config {
            InstallAction::Created
        } else {
            InstallAction::Modified
        }
    };
    let legacy_agents_action =
        remove_legacy_agents_imports(opencode_folder_path, &profile_paths, is_dry_run)?;

    Ok(combine_install_actions(config_action, legacy_agents_action))
}

pub fn uninstall_opencode(
    opencode_folder_path: &Path,
    identity_import_line: &str,
    guidance_import_line: &str,
    is_dry_run: bool,
) -> CliResult<UninstallAction> {
    let profile_paths = get_profile_paths_from_imports(identity_import_line, guidance_import_line);
    let config_file_path = get_opencode_config_file_path(opencode_folder_path);
    let config_action = match read_opencode_config(&config_file_path)? {
        None => UninstallAction::Unchanged,
        Some(mut config) => {
            config.content = remove_profile_paths(&config.path, &config.content, &profile_paths)?;
            let is_modified =
                config.original_content_snapshot.content() != Some(config.content.as_str());

            if !is_modified {
                UninstallAction::Unchanged
            } else if is_dry_run {
                UninstallAction::WouldRemove
            } else {
                replace_file_if_unchanged(
                    &config.path,
                    &config.original_content_snapshot,
                    &config.content,
                )?;

                UninstallAction::Removed
            }
        }
    };
    let legacy_agents_action =
        remove_legacy_agents_imports(opencode_folder_path, &profile_paths, is_dry_run)?;

    Ok(combine_uninstall_actions(
        config_action,
        legacy_agents_action,
    ))
}

pub fn get_opencode_status(
    opencode_folder_path: &Path,
    identity_file_path: &Path,
    guidance_file_path: &Path,
) -> OpenCodeStatus {
    let profile_paths = get_profile_paths(identity_file_path, guidance_file_path);
    let config_file_path = get_opencode_config_file_path(opencode_folder_path);
    let maybe_effective_config = match read_opencode_config(&config_file_path) {
        Ok(config) => config,
        Err(error) => return OpenCodeStatus::Invalid(error.to_string()),
    };

    let Some(effective_config) = maybe_effective_config else {
        return OpenCodeStatus::ConfigurationMissing;
    };

    let instruction_paths =
        match get_instruction_paths(&effective_config.path, &effective_config.content) {
            Ok(instruction_paths) => instruction_paths,
            Err(error) => return OpenCodeStatus::Invalid(error.to_string()),
        };

    for (file_name, profile_path) in [
        ("AGENT.md", &profile_paths[0]),
        ("ME.md", &profile_paths[1]),
    ] {
        let count = instruction_paths
            .iter()
            .filter(|instruction_path| *instruction_path == profile_path)
            .count();

        match count {
            0 => return OpenCodeStatus::InstructionMissing(file_name),
            1 => {}
            count => return OpenCodeStatus::Duplicated(file_name, count),
        }
    }

    OpenCodeStatus::Installed
}

fn get_profile_paths(identity_file_path: &Path, guidance_file_path: &Path) -> [String; 2] {
    [
        guidance_file_path.display().to_string(),
        identity_file_path.display().to_string(),
    ]
}

fn get_profile_paths_from_imports(
    identity_import_line: &str,
    guidance_import_line: &str,
) -> [String; 2] {
    [
        guidance_import_line.trim_start_matches('@').to_string(),
        identity_import_line.trim_start_matches('@').to_string(),
    ]
}

fn combine_install_actions(
    config_action: InstallAction,
    legacy_agents_action: LegacyAgentsAction,
) -> InstallAction {
    match config_action {
        InstallAction::Created => InstallAction::Created,
        InstallAction::Modified => InstallAction::Modified,
        InstallAction::WouldCreate => InstallAction::WouldCreate,
        InstallAction::WouldModify => InstallAction::WouldModify,
        InstallAction::Unchanged => match legacy_agents_action {
            LegacyAgentsAction::Modified => InstallAction::Modified,
            LegacyAgentsAction::Unchanged => InstallAction::Unchanged,
            LegacyAgentsAction::WouldModify => InstallAction::WouldModify,
        },
    }
}

fn combine_uninstall_actions(
    config_action: UninstallAction,
    legacy_agents_action: LegacyAgentsAction,
) -> UninstallAction {
    match config_action {
        UninstallAction::Removed => UninstallAction::Removed,
        UninstallAction::WouldRemove => UninstallAction::WouldRemove,
        UninstallAction::Unchanged => match legacy_agents_action {
            LegacyAgentsAction::Modified => UninstallAction::Removed,
            LegacyAgentsAction::Unchanged => UninstallAction::Unchanged,
            LegacyAgentsAction::WouldModify => UninstallAction::WouldRemove,
        },
    }
}

fn remove_legacy_agents_imports(
    opencode_folder_path: &Path,
    profile_paths: &[String; 2],
    is_dry_run: bool,
) -> CliResult<LegacyAgentsAction> {
    let agents_file_path = opencode_folder_path.join("AGENTS.md");
    let import_lines = profile_paths
        .iter()
        .map(|profile_path| format!("@{profile_path}"))
        .collect::<Vec<_>>();
    let original_content_snapshot = read_file_content_snapshot(&agents_file_path)?;
    let Some(existing_content) = original_content_snapshot.content() else {
        return Ok(LegacyAgentsAction::Unchanged);
    };

    let next_content = remove_exact_import_lines(existing_content, &import_lines);

    if existing_content == next_content {
        return Ok(LegacyAgentsAction::Unchanged);
    }

    if is_dry_run {
        return Ok(LegacyAgentsAction::WouldModify);
    }

    replace_file_if_unchanged(&agents_file_path, &original_content_snapshot, &next_content)?;

    Ok(LegacyAgentsAction::Modified)
}

fn remove_exact_import_lines(content: &str, import_lines: &[String]) -> String {
    let retained_lines = content
        .split_inclusive('\n')
        .filter(|line| {
            let normalized_line = line.trim_end_matches('\n').trim_end_matches('\r');

            !import_lines
                .iter()
                .any(|import_line| import_line == normalized_line)
        })
        .collect::<String>();

    if content.ends_with('\n') {
        retained_lines
    } else {
        retained_lines.trim_end_matches('\n').to_string()
    }
}

fn read_opencode_config(config_file_path: &Path) -> CliResult<Option<OpenCodeConfig>> {
    let original_content_snapshot = read_file_content_snapshot(config_file_path)?;
    let Some(original_content) = original_content_snapshot.content() else {
        return Ok(None);
    };
    let content = original_content.to_string();
    validate_config(config_file_path, &content)?;

    Ok(Some(OpenCodeConfig {
        path: config_file_path.to_path_buf(),
        original_content_snapshot,
        content,
    }))
}

fn remove_profile_paths(
    config_file_path: &Path,
    content: &str,
    profile_paths: &[String],
) -> CliResult<String> {
    if content.is_empty() {
        return Ok(content.to_string());
    }

    let config_root = parse_config(config_file_path, content)?;
    let config_object = get_existing_config_object(config_file_path, &config_root)?;
    let Some(instructions) = get_instructions(config_file_path, &config_object)? else {
        return Ok(content.to_string());
    };

    for instruction in instructions.elements() {
        let maybe_instruction_path = get_instruction_path(config_file_path, &instruction)?;

        if profile_paths.contains(&maybe_instruction_path) {
            instruction.remove();
        }
    }

    Ok(config_root.to_string())
}

fn add_profile_paths(
    config_file_path: &Path,
    content: &str,
    profile_paths: &[String],
) -> CliResult<String> {
    let config_root = parse_config(config_file_path, content)?;
    let config_object = get_config_object_for_add(config_file_path, content, &config_root)?;
    let instructions = match get_instructions(config_file_path, &config_object)? {
        Some(instructions) => instructions,
        None => config_object
            .array_value_or_create(INSTRUCTIONS_PROPERTY_NAME)
            .ok_or_else(|| {
                get_invalid_config_error(
                    config_file_path,
                    "instructions must be an array of string paths",
                )
            })?,
    };

    for profile_path in profile_paths {
        instructions.append(profile_path.clone().into());
    }

    Ok(config_root.to_string())
}

fn normalize_profile_paths(
    config_file_path: &Path,
    content: &str,
    profile_paths: &[String; 2],
) -> CliResult<String> {
    let mut normalized_content = content.to_string();

    for profile_path in profile_paths {
        let instruction_paths = get_instruction_paths(config_file_path, &normalized_content)?;
        let count = instruction_paths
            .iter()
            .filter(|instruction_path| *instruction_path == profile_path)
            .count();

        if count > 1 {
            normalized_content = remove_profile_paths(
                config_file_path,
                &normalized_content,
                std::slice::from_ref(profile_path),
            )?;
        }

        if count != 1 {
            normalized_content = add_profile_paths(
                config_file_path,
                &normalized_content,
                std::slice::from_ref(profile_path),
            )?;
        }
    }

    Ok(normalized_content)
}

fn get_instruction_paths(config_file_path: &Path, content: &str) -> CliResult<Vec<String>> {
    if content.is_empty() {
        return Ok(Vec::new());
    }

    let config_root = parse_config(config_file_path, content)?;
    let config_object = get_existing_config_object(config_file_path, &config_root)?;
    let maybe_instructions = get_instructions(config_file_path, &config_object)?;

    match maybe_instructions {
        Some(instructions) => instructions
            .elements()
            .into_iter()
            .map(|instruction| get_instruction_path(config_file_path, &instruction))
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn parse_config(config_file_path: &Path, content: &str) -> CliResult<CstRootNode> {
    CstRootNode::parse(content, &get_jsonc_parse_options())
        .map_err(|error| get_invalid_config_error(config_file_path, &error.to_string()))
}

fn validate_config(config_file_path: &Path, content: &str) -> CliResult<()> {
    if content.is_empty() {
        return Ok(());
    }

    let config_root = parse_config(config_file_path, content)?;
    let config_object = get_existing_config_object(config_file_path, &config_root)?;
    let maybe_instructions = get_instructions(config_file_path, &config_object)?;

    if let Some(instructions) = maybe_instructions {
        for instruction in instructions.elements() {
            get_instruction_path(config_file_path, &instruction)?;
        }
    }

    Ok(())
}

fn get_existing_config_object(
    config_file_path: &Path,
    config_root: &CstRootNode,
) -> CliResult<CstObject> {
    config_root.object_value().ok_or_else(|| {
        get_invalid_config_error(config_file_path, "configuration root must be an object")
    })
}

fn get_config_object_for_add(
    config_file_path: &Path,
    content: &str,
    config_root: &CstRootNode,
) -> CliResult<CstObject> {
    if content.is_empty() {
        return Ok(config_root.object_value_or_set());
    }

    get_existing_config_object(config_file_path, config_root)
}

fn get_instructions(
    config_file_path: &Path,
    config_object: &CstObject,
) -> CliResult<Option<jsonc_parser::cst::CstArray>> {
    let instruction_properties = config_object
        .properties()
        .into_iter()
        .filter(|property| {
            property
                .name()
                .and_then(|name| name.decoded_value().ok())
                .is_some_and(|name| name == INSTRUCTIONS_PROPERTY_NAME)
        })
        .collect::<Vec<_>>();

    if instruction_properties.len() > 1 {
        return Err(get_invalid_config_error(
            config_file_path,
            "configuration contains duplicate instructions properties",
        ));
    }

    match instruction_properties.as_slice() {
        [] => Ok(None),
        [instruction_property] => instruction_property.array_value().map(Some).ok_or_else(|| {
            get_invalid_config_error(
                config_file_path,
                "instructions must be an array of string paths",
            )
        }),
        _ => unreachable!(),
    }
}

fn get_instruction_path(config_file_path: &Path, instruction: &CstNode) -> CliResult<String> {
    instruction
        .as_string_lit()
        .ok_or_else(|| {
            get_invalid_config_error(
                config_file_path,
                "instructions must contain only string paths",
            )
        })?
        .decoded_value()
        .map_err(|error| get_invalid_config_error(config_file_path, &error.to_string()))
}

fn get_jsonc_parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: true,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

fn get_invalid_config_error(config_file_path: &Path, message: &str) -> Box<dyn Error> {
    format!(
        "OpenCode configuration {} is invalid: {message}",
        config_file_path.display()
    )
    .into()
}
