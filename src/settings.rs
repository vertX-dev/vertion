use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::filter::{parse_version, FilterMode, IncludeEntry, IncrementLevel};
use crate::parser::{parse_condition_token, MarkerCondition};

pub const DEFAULT_CONFIG_NAME: &str = "vertion.cfg";
/// Older config name, still read (and written back to) if present so existing
/// projects don't break on upgrade.
pub const LEGACY_CONFIG_NAME: &str = "vertion.toml";

/// A named condition. Normally exactly one source is set; when several are,
/// precedence is `cmd` > `global` > `bool` (see `conditions::resolve_one`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConditionDef {
    /// Name of a condition in the global (user-level) config to defer to.
    /// If that global condition doesn't exist, falls back to `bool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<String>,
    /// Literal value. The fallback when no `cmd`/`global` applies. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bool: Option<bool>,
    /// Shell command; exit status 0 means true. Empty string counts as unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
}

/// User-level config (`~/.vertion/vertion.cfg`, or `$VERTION_GLOBAL_CONFIG`).
/// Only holds conditions today.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub conditions: BTreeMap<String, ConditionDef>,
}

/// Path of the user-level global config. `$VERTION_GLOBAL_CONFIG` overrides it
/// (also what the tests use so they never touch a real home directory).
pub fn global_config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VERTION_GLOBAL_CONFIG") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".vertion").join(DEFAULT_CONFIG_NAME)
}

pub fn load_global() -> Result<GlobalConfig, SettingsError> {
    let p = global_config_path();
    if !p.exists() {
        return Ok(GlobalConfig::default());
    }
    let text = fs::read_to_string(&p)?;
    Ok(toml::from_str(&text)?)
}

pub fn save_global(cfg: &GlobalConfig) -> Result<PathBuf, SettingsError> {
    let p = global_config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    fs::write(&p, text)?;
    Ok(p)
}

/// Whole-file version assignment: a concrete version (with optional tags), or
/// `EXC` (always exclude — tags are irrelevant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileVersionSpec {
    At {
        version: Version,
        tags: Vec<String>,
        conditions: Vec<MarkerCondition>,
    },
    Exclude,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertionConfig {
    pub project: ProjectSection,
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub last: LastSection,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileSection>,
    #[serde(default, rename = "include")]
    pub include: Vec<IncludeEntryConfig>,
    /// Whole-file version assignments for files that can't carry comment markers
    /// (images, JSON, binaries). A file is excluded from the build when its
    /// assigned version fails the active filter; otherwise it copies as-is.
    #[serde(default, rename = "files")]
    pub files: Vec<FileVersion>,
    /// Named conditions referenced by `{name}` on marker tags.
    #[serde(default)]
    pub conditions: BTreeMap<String, ConditionDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileVersion {
    /// Path relative to the input directory (forward slashes; leading `./` optional).
    pub path: String,
    pub version: String,
    /// Optional tags, filtered the same way as in-code block tags (`--tag`, OR-logic).
    /// Ignored for `version = "EXC"`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional conditions, gating the file the same way `{cond}` gates a marker.
    /// Prefix a name with `!` to negate it. Ignored for `version = "EXC"`.
    #[serde(default)]
    pub conditions: Vec<String>,
}

/// Normalize a path for matching: forward slashes, no leading `./`.
pub fn normalize_path_key(s: &str) -> String {
    s.replace('\\', "/").trim_start_matches("./").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncludeEntryConfig {
    pub from: String,
    pub to: String,
}

impl IncludeEntryConfig {
    pub fn parse(&self) -> Result<IncludeEntry, SettingsError> {
        let from = parse_version(&self.from).map_err(|e| SettingsError(e.to_string()))?;
        let to = parse_version(&self.to).map_err(|e| SettingsError(e.to_string()))?;
        if from > to {
            return Err(SettingsError(format!(
                "include entry {} > {}",
                self.from, self.to
            )));
        }
        Ok(IncludeEntry { from, to })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSection {
    pub version: String,
    #[serde(default = "default_input")]
    pub input: PathBuf,
    #[serde(default = "default_output")]
    pub output: PathBuf,
    #[serde(default)]
    pub ignore: Vec<PathBuf>,
    /// Tags active when neither `--tag` nor a profile's `tags` is given.
    /// Empty means no tags are active, so all tagged code and files are skipped.
    /// Use `["*"]` to admit every tag.
    #[serde(default)]
    pub default_tags: Vec<String>,
    /// Tag preference order, most important first. Breaks ties when several
    /// variants of the same file match at the same version — without it, two
    /// equally specific matches are an ambiguity error.
    #[serde(default)]
    pub tag_priority: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSection {
    #[serde(default = "default_increment")]
    pub increment: String,
}

// Manual Default so a config omitting the entire `[build]` table still gets the
// documented "minor" increment (a derived Default would leave it as "").
impl Default for BuildSection {
    fn default() -> Self {
        BuildSection {
            increment: default_increment(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LastSection {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub dev: bool,
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub mode: String, // "cumulative" | "range" | "only" | "include"
    #[serde(default)]
    pub range_from: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub profile: String,
    /// Last wrap mode used: "temp" / "perm" / empty for disabled.
    #[serde(default)]
    pub wrap: String,
    #[serde(default)]
    pub wrap_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSection {
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub ignore: Vec<PathBuf>,
    pub increment: Option<String>,
    /// Post-build commands executed **in the build output folder**.
    #[serde(default)]
    pub run: Vec<String>,
    /// Post-build commands executed **in the directory vertion was invoked from**.
    /// Runs after `run`.
    #[serde(default)]
    pub run_here: Vec<String>,
    /// Default tag filter for builds using this profile (CLI `--tag` replaces it when given).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Wrap mode: "temp" or "perm". `None` disables wrap.
    pub wrap: Option<String>,
    /// Wrap folder name. Defaults to `.vertion_wrap`.
    pub wrap_name: Option<String>,
}

fn default_input() -> PathBuf {
    PathBuf::from("./src")
}
fn default_output() -> PathBuf {
    PathBuf::from("./build")
}
fn default_increment() -> String {
    "minor".into()
}

impl VertionConfig {
    pub fn default_template() -> Self {
        VertionConfig {
            project: ProjectSection {
                version: "0.1.0".into(),
                input: default_input(),
                output: default_output(),
                ignore: vec![PathBuf::from("./build"), PathBuf::from("./node_modules")],
                default_tags: Vec::new(),
                tag_priority: Vec::new(),
            },
            build: BuildSection {
                increment: default_increment(),
            },
            last: LastSection::default(),
            profiles: BTreeMap::new(),
            include: Vec::new(),
            files: Vec::new(),
            conditions: BTreeMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn increment_level(&self) -> IncrementLevel {
        IncrementLevel::parse(&self.build.increment).unwrap_or(IncrementLevel::Minor)
    }

    /// Apply a profile's overrides on top of the project section, returning a
    /// `(input, output, ignore, increment)` tuple ready for the builder.
    pub fn resolve_profile(&self, name: Option<&str>) -> Result<ResolvedSettings, SettingsError> {
        let mut input = self.project.input.clone();
        let mut output = self.project.output.clone();
        let mut ignore = self.project.ignore.clone();
        let mut increment = self.build.increment.clone();
        let mut run: Vec<String> = Vec::new();
        let mut run_here: Vec<String> = Vec::new();
        // Project-level default; a profile's own `tags` replaces it when set.
        let mut tags: Vec<String> = self.project.default_tags.clone();
        let mut wrap: Option<String> = None;
        let mut wrap_name: Option<String> = None;

        if let Some(n) = name {
            let prof = self.profiles.get(n).ok_or_else(|| {
                SettingsError(format!("profile `{}` not found in vertion.cfg", n))
            })?;
            if let Some(p) = &prof.input {
                input = p.clone();
            }
            if let Some(p) = &prof.output {
                output = p.clone();
            }
            if !prof.ignore.is_empty() {
                ignore = prof.ignore.clone();
            }
            if let Some(i) = &prof.increment {
                if IncrementLevel::parse(i).is_none() {
                    return Err(SettingsError(format!(
                        "profile `{}` has invalid increment `{}`",
                        n, i
                    )));
                }
                increment = i.clone();
            }
            run = prof.run.clone();
            run_here = prof.run_here.clone();
            tags = prof.tags.clone();
            wrap = prof.wrap.clone();
            wrap_name = prof.wrap_name.clone();
        }

        if IncrementLevel::parse(&increment).is_none() {
            return Err(SettingsError(format!(
                "invalid build.increment `{}`",
                increment
            )));
        }
        Ok(ResolvedSettings {
            input,
            output,
            ignore,
            increment: IncrementLevel::parse(&increment).unwrap(),
            profile: name.map(|s| s.to_string()),
            run,
            run_here,
            tags,
            tag_priority: self.project.tag_priority.clone(),
            wrap,
            wrap_name,
        })
    }

    /// Parse the persisted include list into runtime entries.
    pub fn include_entries(&self) -> Result<Vec<IncludeEntry>, SettingsError> {
        self.include.iter().map(|c| c.parse()).collect()
    }

    /// Parse `[[files]]` into `(normalized_path, spec)` pairs.
    pub fn file_versions(&self) -> Result<Vec<(String, FileVersionSpec)>, SettingsError> {
        self.files
            .iter()
            .map(|f| {
                let spec = if f.version.eq_ignore_ascii_case("EXC") {
                    FileVersionSpec::Exclude
                } else {
                    let conditions = f
                        .conditions
                        .iter()
                        .map(|c| parse_condition_token(c).map_err(SettingsError))
                        .collect::<Result<Vec<_>, _>>()?;
                    FileVersionSpec::At {
                        version: parse_version(&f.version)
                            .map_err(|e| SettingsError(e.to_string()))?,
                        tags: f.tags.clone(),
                        conditions,
                    }
                };
                Ok((normalize_path_key(&f.path), spec))
            })
            .collect()
    }
}

pub struct ResolvedSettings {
    pub input: PathBuf,
    pub output: PathBuf,
    pub ignore: Vec<PathBuf>,
    pub increment: IncrementLevel,
    pub profile: Option<String>,
    pub run: Vec<String>,
    pub run_here: Vec<String>,
    pub tags: Vec<String>,
    pub tag_priority: Vec<String>,
    pub wrap: Option<String>,
    pub wrap_name: Option<String>,
}

#[derive(Debug)]
pub struct SettingsError(pub String);

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for SettingsError {}
impl From<io::Error> for SettingsError {
    fn from(e: io::Error) -> Self {
        SettingsError(e.to_string())
    }
}
impl From<toml::de::Error> for SettingsError {
    fn from(e: toml::de::Error) -> Self {
        SettingsError(format!("toml parse error: {}", e))
    }
}
impl From<toml::ser::Error> for SettingsError {
    fn from(e: toml::ser::Error) -> Self {
        SettingsError(format!("toml serialize error: {}", e))
    }
}

/// Where a new config is written.
pub fn config_path(project_root: &Path) -> PathBuf {
    project_root.join(DEFAULT_CONFIG_NAME)
}

/// The config file to read/write: `.cfg` if present, else legacy `.toml` if
/// present, else the default `.cfg` path (for creation).
pub fn active_config_path(project_root: &Path) -> PathBuf {
    let cfg = config_path(project_root);
    if cfg.exists() {
        return cfg;
    }
    let legacy = project_root.join(LEGACY_CONFIG_NAME);
    if legacy.exists() {
        return legacy;
    }
    cfg
}

pub fn load_or_default(project_root: &Path) -> Result<VertionConfig, SettingsError> {
    let p = active_config_path(project_root);
    if !p.exists() {
        return Ok(VertionConfig::default_template());
    }
    let text = fs::read_to_string(&p)?;
    let cfg: VertionConfig = toml::from_str(&text)?;
    Ok(cfg)
}

#[allow(dead_code)]
pub fn load(project_root: &Path) -> Result<Option<VertionConfig>, SettingsError> {
    let p = active_config_path(project_root);
    if !p.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p)?;
    let cfg: VertionConfig = toml::from_str(&text)?;
    Ok(Some(cfg))
}

pub fn save(cfg: &VertionConfig, project_root: &Path) -> Result<(), SettingsError> {
    let text = toml::to_string_pretty(cfg)?;
    fs::write(active_config_path(project_root), text)?;
    Ok(())
}

pub fn write_default_template(project_root: &Path) -> Result<PathBuf, SettingsError> {
    let existing = active_config_path(project_root);
    if existing.exists() {
        return Err(SettingsError(format!(
            "{} already exists",
            existing.display()
        )));
    }
    let p = config_path(project_root);
    let cfg = VertionConfig::default_template();
    let body = toml::to_string_pretty(&cfg)?;
    let commented = format!(
        "# Vertion project configuration.\n\
         # Generated by `vertion init`. See README for details.\n\n\
         {}",
        body
    );
    fs::write(&p, commented)?;
    Ok(p)
}

#[allow(clippy::too_many_arguments)]
pub fn save_last(
    project_root: &Path,
    filter: &FilterMode,
    dev: bool,
    auto: bool,
    tags: &[String],
    profile: Option<&str>,
    wrap: Option<&str>,
    wrap_name: Option<&str>,
) -> Result<(), SettingsError> {
    let mut cfg = load_or_default(project_root)?;
    cfg.last = LastSection {
        version: filter.upper().to_string(),
        dev,
        auto,
        mode: filter.name().to_string(),
        range_from: match filter {
            FilterMode::Range(from, _) => from.to_string(),
            _ => String::new(),
        },
        tags: tags.to_vec(),
        profile: profile.unwrap_or("").to_string(),
        wrap: wrap.unwrap_or("").to_string(),
        wrap_name: wrap_name.unwrap_or("").to_string(),
    };
    save(&cfg, project_root)
}

pub fn save_include(project_root: &Path, entries: &[IncludeEntry]) -> Result<(), SettingsError> {
    let mut cfg = load_or_default(project_root)?;
    cfg.include = entries
        .iter()
        .map(|e| IncludeEntryConfig {
            from: e.from.to_string(),
            to: e.to.to_string(),
        })
        .collect();
    save(&cfg, project_root)
}

pub fn save_version(project_root: &Path, version: &str) -> Result<(), SettingsError> {
    let mut cfg = load_or_default(project_root)?;
    cfg.project.version = version.to_string();
    save(&cfg, project_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::parse_filter;

    fn tmp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("vertion-settings-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn load_or_default_when_missing() {
        let dir = tmp("missing");
        let cfg = load_or_default(&dir).unwrap();
        assert_eq!(cfg.project.version, "0.1.0");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tmp("roundtrip");
        let mut cfg = VertionConfig::default_template();
        cfg.project.version = "2.5.0".into();
        cfg.profiles.insert(
            "prod".into(),
            ProfileSection {
                input: Some(PathBuf::from("./src")),
                output: Some(PathBuf::from("./build/prod")),
                ignore: vec![PathBuf::from("tests")],
                increment: Some("minor".into()),
                run: Vec::new(),
                run_here: Vec::new(),
                tags: Vec::new(),
                wrap: None,
                wrap_name: None,
            },
        );
        save(&cfg, &dir).unwrap();
        let loaded = load_or_default(&dir).unwrap();
        assert_eq!(loaded.project.version, "2.5.0");
        let resolved = loaded.resolve_profile(Some("prod")).unwrap();
        assert_eq!(resolved.output, PathBuf::from("./build/prod"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_last_records_mode() {
        let dir = tmp("last");
        let filter = parse_filter(&[String::from("1.2")]).unwrap();
        save_last(&dir, &filter, true, false, &[], None, None, None).unwrap();
        let loaded = load_or_default(&dir).unwrap();
        assert_eq!(loaded.last.mode, "cumulative");
        assert_eq!(loaded.last.version, "1.2.0");
        assert!(loaded.last.dev);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_resolves_both_run_lists() {
        let mut cfg = VertionConfig::default_template();
        cfg.profiles.insert(
            "prod".into(),
            ProfileSection {
                run: vec!["npm run build".into()],
                run_here: vec!["git add build".into()],
                ..Default::default()
            },
        );
        let r = cfg.resolve_profile(Some("prod")).unwrap();
        assert_eq!(r.run, vec!["npm run build".to_string()]);
        assert_eq!(r.run_here, vec!["git add build".to_string()]);
        // No profile → both empty.
        let none = cfg.resolve_profile(None).unwrap();
        assert!(none.run.is_empty() && none.run_here.is_empty());
    }

    #[test]
    fn missing_profile_errors() {
        let cfg = VertionConfig::default_template();
        assert!(cfg.resolve_profile(Some("nope")).is_err());
    }
}
