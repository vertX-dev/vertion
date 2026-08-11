//! Named conditions attached to marker tags (`//version [stable{imagesInStable}]`).
//!
//! A condition resolves to a single boolean per build. Definitions live in
//! `[conditions.NAME]` tables — in the project `vertion.cfg`, and/or in the
//! user-level global config (`~/.vertion/vertion.cfg`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::runner::{shell_test, BuildEnv};
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
///
/// `env` carries the build's `VERTION_*` variables so a `cmd` probe can test the
/// build it is about to gate (e.g. whether the output folder already exists).
/// It is empty outside a build.
pub fn resolve_one(
    def: &ConditionDef,
    global: &GlobalConfig,
    cwd: &Path,
    env: &BuildEnv,
) -> ResolvedCondition {
    if let Some(cmd) = non_empty(def.cmd.as_ref()) {
        let value = shell_test(cmd, cwd, env).unwrap_or(false);
        return ResolvedCondition {
            value,
            source: format!("cmd: {}", cmd),
        };
    }
    if let Some(name) = non_empty(def.global.as_ref()) {
        if let Some(g) = global.conditions.get(name) {
            if let Some(cmd) = non_empty(g.cmd.as_ref()) {
                let value = shell_test(cmd, cwd, env).unwrap_or(false);
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
    env: &BuildEnv,
) -> BTreeMap<String, ResolvedCondition> {
    let mut out = BTreeMap::new();
    for (name, def) in project {
        out.insert(name.clone(), resolve_one(def, global, cwd, env));
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
        let mut r = resolve_one(&stripped, global, cwd, env);
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

    /// Resolve outside a build — the common case in these tests.
    fn resolve(d: &ConditionDef, g: &GlobalConfig) -> ResolvedCondition {
        resolve_one(d, g, &cwd(), &BuildEnv::default())
    }

    #[test]
    fn bool_source() {
        let g = GlobalConfig::default();
        assert!(resolve(&def(None, Some(true), None), &g).value);
        assert!(!resolve(&def(None, Some(false), None), &g).value);
        // Nothing set at all → false.
        assert!(!resolve(&def(None, None, None), &g).value);
    }

    #[test]
    fn empty_cmd_and_global_count_as_unset() {
        let g = GlobalConfig::default();
        let r = resolve(&def(Some(""), Some(true), Some("")), &g);
        assert!(r.value);
        assert_eq!(r.source, "bool");
    }

    #[test]
    fn cmd_wins_and_uses_exit_status() {
        let g = GlobalConfig::default();
        let ok = if cfg!(windows) { "exit 0" } else { "true" };
        let bad = if cfg!(windows) { "exit 1" } else { "false" };
        // cmd beats a contradicting bool
        assert!(resolve(&def(None, Some(false), Some(ok)), &g).value);
        assert!(!resolve(&def(None, Some(true), Some(bad)), &g).value);
    }

    #[test]
    fn cmd_probe_sees_the_build_environment() {
        use crate::runner::{var, BuildFacts};
        let g = GlobalConfig::default();
        let env = BuildEnv::new(&BuildFacts {
            root: Path::new("."),
            input: Path::new("./src"),
            output: Path::new("./build/2.5.0"),
            version: "2.5.0",
            mode: "cumulative",
            profile: None,
            tags: &[],
            dev: false,
        });
        #[cfg(windows)]
        let probe = "if \"%VERTION_VERSION%\"==\"2.5.0\" (exit 0) else (exit 1)";
        #[cfg(not(windows))]
        let probe = "[ \"$VERTION_VERSION\" = \"2.5.0\" ]";
        assert!(resolve_one(&def(None, Some(false), Some(probe)), &g, &cwd(), &env).value);
        assert_eq!(env.get(var::VERSION), Some("2.5.0"));
    }

    #[test]
    fn global_reference_resolves_and_falls_back() {
        let mut g = GlobalConfig::default();
        g.conditions
            .insert("apiReleased".into(), def(None, Some(true), None));
        // Defined globally → uses the global value, not the local bool.
        let r = resolve(&def(Some("apiReleased"), Some(false), None), &g);
        assert!(r.value);
        // Not defined globally → falls back to the local bool.
        let r2 = resolve(&def(Some("notThere"), Some(false), None), &g);
        assert!(!r2.value);
        assert!(r2.source.contains("undefined"));
    }

    #[test]
    fn global_only_conditions_are_visible_to_builds() {
        let mut g = GlobalConfig::default();
        g.conditions
            .insert("apiReleased".into(), def(None, Some(true), None));
        let project = BTreeMap::new();
        let all = resolve_all(&project, &g, &cwd(), &BuildEnv::default());
        assert!(all.get("apiReleased").unwrap().value);
    }

    #[test]
    fn project_definition_shadows_global() {
        let mut g = GlobalConfig::default();
        g.conditions.insert("x".into(), def(None, Some(true), None));
        let mut project = BTreeMap::new();
        project.insert("x".to_string(), def(None, Some(false), None));
        let all = resolve_all(&project, &g, &cwd(), &BuildEnv::default());
        assert!(!all.get("x").unwrap().value);
    }
}
