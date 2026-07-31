//! Named conditions attached to marker tags (`//version [stable{imagesInStable}]`).
//!
//! A condition resolves to a single boolean per build. Definitions live in
//! `[conditions.NAME]` tables — in the project `vertion.cfg`, and/or in the
//! user-level global config (`~/.vertion/vertion.cfg`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::runner::shell_test;
use crate::settings::{ConditionDef, GlobalConfig};

#[derive(Debug, Clone)]
pub struct ResolvedCondition {
    pub value: bool,
    /// Human-readable description of which source decided the value.
    pub source: String,
}

// The condition *syntax* types (`MarkerCondition`, `parse_condition_token`,
// `conditions_pass`) live in `parser.rs` so they stay part of the pure library
// surface. This module owns only resolution, which needs config + shell access.

/// Non-empty-after-trim, else `None`. `cmd = ''` / `global = ''` count as unset.
fn non_empty(s: Option<&String>) -> Option<&str> {
    s.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// Resolve a single definition. Precedence: `cmd` > `global` > `bool`.
///
/// A `global` reference that names an undefined global condition falls back to
/// the local `bool` — that's the "waiting on something external" case: the
/// condition reads false until the global is defined. Global entries are
/// resolved one level deep only (their own `global` field is ignored), so
/// reference cycles are impossible by construction.
pub fn resolve_one(def: &ConditionDef, global: &GlobalConfig, cwd: &Path) -> ResolvedCondition {
    if let Some(cmd) = non_empty(def.cmd.as_ref()) {
        let value = shell_test(cmd, cwd).unwrap_or(false);
        return ResolvedCondition {
            value,
            source: format!("cmd: {}", cmd),
        };
    }
    if let Some(name) = non_empty(def.global.as_ref()) {
        if let Some(g) = global.conditions.get(name) {
            if let Some(cmd) = non_empty(g.cmd.as_ref()) {
                let value = shell_test(cmd, cwd).unwrap_or(false);
                return ResolvedCondition {
                    value,
                    source: format!("global:{} → cmd: {}", name, cmd),
                };
            }
            let value = g.bool.unwrap_or(false);
            return ResolvedCondition {
                value,
                source: format!("global:{} → bool", name),
            };
        }
        return ResolvedCondition {
            value: def.bool.unwrap_or(false),
            source: format!("global:{} (undefined) → bool", name),
        };
    }
    ResolvedCondition {
        value: def.bool.unwrap_or(false),
        source: "bool".to_string(),
    }
}

/// Resolve every condition visible to a build: the project's own definitions,
/// plus any global-only ones it didn't shadow.
pub fn resolve_all(
    project: &BTreeMap<String, ConditionDef>,
    global: &GlobalConfig,
    cwd: &Path,
) -> BTreeMap<String, ResolvedCondition> {
    let mut out = BTreeMap::new();
    for (name, def) in project {
        out.insert(name.clone(), resolve_one(def, global, cwd));
    }
    for (name, def) in &global.conditions {
        if out.contains_key(name) {
            continue; // project definition wins
        }
        // Global entries resolve without a nested global lookup.
        let stripped = ConditionDef {
            global: None,
            ..def.clone()
        };
        let mut r = resolve_one(&stripped, global, cwd);
        r.source = format!("global:{} ({})", name, r.source);
        out.insert(name.clone(), r);
    }
    out
}

/// Flatten to the `(name, value)` pairs the parser/builder consume.
pub fn to_pairs(resolved: &BTreeMap<String, ResolvedCondition>) -> Vec<(String, bool)> {
    resolved.iter().map(|(k, v)| (k.clone(), v.value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(global: Option<&str>, b: Option<bool>, cmd: Option<&str>) -> ConditionDef {
        ConditionDef {
            global: global.map(|s| s.to_string()),
            bool: b,
            cmd: cmd.map(|s| s.to_string()),
        }
    }

    fn cwd() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn bool_source() {
        let g = GlobalConfig::default();
        assert!(resolve_one(&def(None, Some(true), None), &g, &cwd()).value);
        assert!(!resolve_one(&def(None, Some(false), None), &g, &cwd()).value);
        // Nothing set at all → false.
        assert!(!resolve_one(&def(None, None, None), &g, &cwd()).value);
    }

    #[test]
    fn empty_cmd_and_global_count_as_unset() {
        let g = GlobalConfig::default();
        let r = resolve_one(&def(Some(""), Some(true), Some("")), &g, &cwd());
        assert!(r.value);
        assert_eq!(r.source, "bool");
    }

    #[test]
    fn cmd_wins_and_uses_exit_status() {
        let g = GlobalConfig::default();
        let ok = if cfg!(windows) { "exit 0" } else { "true" };
        let bad = if cfg!(windows) { "exit 1" } else { "false" };
        // cmd beats a contradicting bool
        assert!(resolve_one(&def(None, Some(false), Some(ok)), &g, &cwd()).value);
        assert!(!resolve_one(&def(None, Some(true), Some(bad)), &g, &cwd()).value);
    }

    #[test]
    fn global_reference_resolves_and_falls_back() {
        let mut g = GlobalConfig::default();
        g.conditions
            .insert("apiReleased".into(), def(None, Some(true), None));
        // Defined globally → uses the global value, not the local bool.
        let r = resolve_one(&def(Some("apiReleased"), Some(false), None), &g, &cwd());
        assert!(r.value);
        // Not defined globally → falls back to the local bool.
        let r2 = resolve_one(&def(Some("notThere"), Some(false), None), &g, &cwd());
        assert!(!r2.value);
        assert!(r2.source.contains("undefined"));
    }

    #[test]
    fn global_only_conditions_are_visible_to_builds() {
        let mut g = GlobalConfig::default();
        g.conditions
            .insert("apiReleased".into(), def(None, Some(true), None));
        let project = BTreeMap::new();
        let all = resolve_all(&project, &g, &cwd());
        assert!(all.get("apiReleased").unwrap().value);
    }

    #[test]
    fn project_definition_shadows_global() {
        let mut g = GlobalConfig::default();
        g.conditions.insert("x".into(), def(None, Some(true), None));
        let mut project = BTreeMap::new();
        project.insert("x".to_string(), def(None, Some(false), None));
        let all = resolve_all(&project, &g, &cwd());
        assert!(!all.get("x").unwrap().value);
    }
}
