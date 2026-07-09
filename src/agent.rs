use std::path::{Path, PathBuf};

use clap::ValueEnum;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentKind {
    Codex,
    Claude,
    Opencode,
}

#[derive(Clone, Copy)]
pub struct AgentTarget {
    pub kind: AgentKind,
    pub name: &'static str,
    pub folder_relative_path: &'static str,
    pub file_name: &'static str,
}

pub const AGENT_TARGETS: [AgentTarget; 3] = [
    AgentTarget {
        kind: AgentKind::Codex,
        name: "Codex",
        folder_relative_path: ".codex",
        file_name: "AGENTS.md",
    },
    AgentTarget {
        kind: AgentKind::Claude,
        name: "Claude",
        folder_relative_path: ".claude",
        file_name: "CLAUDE.md",
    },
    AgentTarget {
        kind: AgentKind::Opencode,
        name: "OpenCode",
        folder_relative_path: ".config/opencode",
        file_name: "AGENTS.md",
    },
];

pub fn get_agent_folder_path(home_path: &Path, agent_target: &AgentTarget) -> PathBuf {
    home_path.join(agent_target.folder_relative_path)
}

pub fn does_agent_match_filter(
    agent_target: AgentTarget,
    maybe_agent_kind: Option<AgentKind>,
) -> bool {
    match maybe_agent_kind {
        Some(agent_kind) => agent_target.kind == agent_kind,
        None => true,
    }
}
