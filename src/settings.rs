use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filter::{parse_version, FilterMode, IncludeEntry, IncrementLevel};

pub const DEFAULT_CONFIG_NAME: &str = "vertion.toml";

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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildSection {
    #[serde(default = "default_increment")]
    pub increment: String,
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
    pub mode: String, // "cumulative" | "range" | "only"
    #[serde(default)]
    pub range_from: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub profile: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSection {
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub ignore: Vec<PathBuf>,
    pub increment: Option<String>,
    #[serde(default)]
    pub run: Vec<String>,
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
            },
            build: BuildSection {
                increment: default_increment(),
            },
            last: LastSection::default(),
            profiles: BTreeMap::new(),
            include: Vec::new(),
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

        if let Some(n) = name {
            let prof = self.profiles.get(n).ok_or_else(|| {
                SettingsError(format!("profile `{}` not found in vertion.toml", n))
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
        })
    }

    /// Parse the persisted include list into runtime entries.
    pub fn include_entries(&self) -> Result<Vec<IncludeEntry>, SettingsError> {
        self.include.iter().map(|c| c.parse()).collect()
    }
}

pub struct ResolvedSettings {
    pub input: PathBuf,
    pub output: PathBuf,
    pub ignore: Vec<PathBuf>,
    pub increment: IncrementLevel,
    pub profile: Option<String>,
    pub run: Vec<String>,
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

pub fn config_path(project_root: &Path) -> PathBuf {
    project_root.join(DEFAULT_CONFIG_NAME)
}

pub fn load_or_default(project_root: &Path) -> Result<VertionConfig, SettingsError> {
    let p = config_path(project_root);
    if !p.exists() {
        return Ok(VertionConfig::default_template());
    }
    let text = fs::read_to_string(&p)?;
    let cfg: VertionConfig = toml::from_str(&text)?;
    Ok(cfg)
}

#[allow(dead_code)]
pub fn load(project_root: &Path) -> Result<Option<VertionConfig>, SettingsError> {
    let p = config_path(project_root);
    if !p.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p)?;
    let cfg: VertionConfig = toml::from_str(&text)?;
    Ok(Some(cfg))
}

pub fn save(cfg: &VertionConfig, project_root: &Path) -> Result<(), SettingsError> {
    let text = toml::to_string_pretty(cfg)?;
    fs::write(config_path(project_root), text)?;
    Ok(())
}

pub fn write_default_template(project_root: &Path) -> Result<PathBuf, SettingsError> {
    let p = config_path(project_root);
    if p.exists() {
        return Err(SettingsError(format!("{} already exists", p.display())));
    }
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

pub fn save_last(
    project_root: &Path,
    filter: &FilterMode,
    dev: bool,
    auto: bool,
    tags: &[String],
    profile: Option<&str>,
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
        save_last(&dir, &filter, true, false, &[], None).unwrap();
        let loaded = load_or_default(&dir).unwrap();
        assert_eq!(loaded.last.mode, "cumulative");
        assert_eq!(loaded.last.version, "1.2.0");
        assert!(loaded.last.dev);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_profile_errors() {
        let cfg = VertionConfig::default_template();
        assert!(cfg.resolve_profile(Some("nope")).is_err());
    }
}
