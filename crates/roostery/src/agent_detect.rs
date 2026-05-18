//! Detect installed AI agent CLIs (CC / Codex / Gemini) on the host.
//!
//! Used by `roostery init` to decide which Stop hook templates to merge.
//! `which::which()` does PATH-walk lookup; not-found is **not** an error
//! (returns `cli_path: None`).
//!
//! See `.codestable/features/2026-05-18-roostery-init/roostery-init-design.md`
//! §2.1.2.

use crate::hooks_merge::AgentKind;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentSpec {
    pub kind: AgentKind,
    /// Executable name on PATH (e.g. `"claude"`).
    pub cli: &'static str,
    /// Hooks file target, with `~` un-expanded (consumer expands via
    /// [`AgentSpec::expanded_hooks_target`]).
    pub hooks_target: &'static str,
}

pub const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        kind: AgentKind::Cc,
        cli: "claude",
        hooks_target: "~/.claude/settings.json",
    },
    AgentSpec {
        kind: AgentKind::Codex,
        cli: "codex",
        hooks_target: "~/.codex/hooks.json",
    },
    AgentSpec {
        kind: AgentKind::Gemini,
        cli: "gemini",
        hooks_target: "~/.gemini/settings.json",
    },
];

impl AgentSpec {
    /// Resolve `~/<rest>` to `<home>/<rest>`. Falls back to literal string
    /// if HOME is unavailable.
    pub fn expanded_hooks_target(self) -> PathBuf {
        if let Some(rest) = self.hooks_target.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(rest);
        }
        PathBuf::from(self.hooks_target)
    }
}

#[derive(Debug, Clone)]
pub struct DetectResult {
    pub spec: AgentSpec,
    /// `Some(path)` if CLI was found on PATH, `None` otherwise (incl. skipped).
    pub cli_path: Option<PathBuf>,
}

impl DetectResult {
    pub fn installed(&self) -> bool {
        self.cli_path.is_some()
    }
}

/// Detect all known agents. `skip` forces those agents to be reported as
/// not installed even if their CLI is on PATH.
pub fn detect_all(skip: &[AgentKind]) -> Vec<DetectResult> {
    AGENTS
        .iter()
        .copied()
        .map(|spec| {
            let cli_path = if skip.contains(&spec.kind) {
                None
            } else {
                which::which(spec.cli).ok()
            };
            DetectResult { spec, cli_path }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_const_has_three_entries() {
        assert_eq!(AGENTS.len(), 3);
        assert_eq!(AGENTS[0].kind, AgentKind::Cc);
        assert_eq!(AGENTS[1].kind, AgentKind::Codex);
        assert_eq!(AGENTS[2].kind, AgentKind::Gemini);
    }

    #[test]
    fn expanded_hooks_target_resolves_tilde() {
        let cc = AGENTS[0];
        let p = cc.expanded_hooks_target();
        assert!(!p.to_string_lossy().starts_with('~'));
        assert!(p.ends_with(".claude/settings.json"));
    }

    #[test]
    fn skip_forces_not_installed_regardless_of_path() {
        let results = detect_all(&[AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini]);
        assert_eq!(results.len(), 3);
        for r in &results {
            assert!(!r.installed(), "skipped agent must report not-installed");
        }
    }

    #[test]
    fn detect_all_returns_three_results_with_correct_order() {
        let results = detect_all(&[]);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].spec.kind, AgentKind::Cc);
        assert_eq!(results[1].spec.kind, AgentKind::Codex);
        assert_eq!(results[2].spec.kind, AgentKind::Gemini);
    }
}
