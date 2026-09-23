use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

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

/// Add/replace (`Some`) or remove (`None`) one condition in the global config,
/// leaving the rest of the file as the user wrote it. Returns the file's path.
pub fn save_global_condition(
    name: &str,
    def: Option<&ConditionDef>,
) -> Result<PathBuf, SettingsError> {
    let p = global_config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    edit_file(
        &p,
        || Ok(String::new()),
        |doc| edit_condition(doc, name, def),
    )?;
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

fn default_template_text() -> Result<String, SettingsError> {
    let body = toml::to_string_pretty(&VertionConfig::default_template())?;
    Ok(format!(
        "# Vertion project configuration.\n\
         # Generated by `vertion init`. See README for details.\n\n\
         {}",
        body
    ))
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
    fs::write(&p, default_template_text()?)?;
    Ok(p)
}

// ---- Writing to an existing config ----
//
// The config is the user's file: it carries their comments, alignment and table
// order. Every writer below edits the parsed document in place and changes only
// the keys it owns, instead of serialising `VertionConfig` back over the file.

/// Apply `edit` to the TOML document in `path` and write it back. Everything
/// `edit` doesn't touch is kept byte-for-byte, and a CRLF file stays CRLF. A
/// missing file starts out as `initial()`.
fn edit_file(
    path: &Path,
    initial: impl FnOnce() -> Result<String, SettingsError>,
    edit: impl FnOnce(&mut DocumentMut) -> Result<(), SettingsError>,
) -> Result<(), SettingsError> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let original = match &existing {
        Some(text) => text.clone(),
        None => initial()?,
    };
    let updated = edit_text(&original, edit)?;
    if existing.as_deref() != Some(updated.as_str()) {
        fs::write(path, updated)?;
    }
    Ok(())
}

fn edit_project_config(
    project_root: &Path,
    edit: impl FnOnce(&mut DocumentMut) -> Result<(), SettingsError>,
) -> Result<(), SettingsError> {
    edit_file(
        &active_config_path(project_root),
        default_template_text,
        edit,
    )
}

fn edit_text(
    original: &str,
    edit: impl FnOnce(&mut DocumentMut) -> Result<(), SettingsError>,
) -> Result<String, SettingsError> {
    let mut doc: DocumentMut = original
        .parse()
        .map_err(|e| SettingsError(format!("toml parse error: {}", e)))?;
    // Comments after the last key render below anything the edit appends. In a
    // file of nothing but comments they are a header, so keep them on top.
    let out = if doc.is_empty() {
        let header = doc.to_string();
        doc.set_trailing("");
        edit(&mut doc)?;
        format!("{}{}", header, doc)
    } else {
        edit(&mut doc)?;
        doc.to_string()
    };
    // toml_edit ends every line it emits with a bare `\n`; restore the file's CRLF.
    Ok(if uses_crlf(original) {
        lf_to_crlf(&out)
    } else {
        out
    })
}

/// Line-ending convention of a file, judged by its first line.
fn uses_crlf(text: &str) -> bool {
    text.find('\n').is_some_and(|i| text[..i].ends_with('\r'))
}

/// Turn every bare `\n` into `\r\n`, leaving existing `\r\n` alone.
fn lf_to_crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// The root-level table `key`, appended at the end of the file when absent.
/// `implicit` suppresses its `[key]` header while it only holds sub-tables.
fn root_table<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
    implicit: bool,
) -> Result<&'a mut dyn TableLike, SettingsError> {
    if !doc.contains_key(key) {
        let mut t = Table::new();
        t.set_implicit(implicit);
        // Without an explicit position a new table renders after whichever
        // table precedes it in key order, which need not be the file's last.
        t.set_position(Some(next_position(doc.as_table())));
        doc.insert(key, Item::Table(t));
    }
    doc.get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| SettingsError(format!("`{}` in the config is not a table", key)))
}

/// A table position past every table in the document.
fn next_position(root: &Table) -> isize {
    let mut max = root.position().unwrap_or(0);
    for_each_table(root, &mut |t| max = max.max(t.position().unwrap_or(0)));
    max + 1
}

/// Every table below `t`, depth-first in key order.
fn for_each_table(t: &Table, f: &mut impl FnMut(&Table)) {
    for (_, item) in t.iter() {
        let children: Vec<&Table> = match item {
            Item::Table(c) => vec![c],
            Item::ArrayOfTables(aot) => aot.iter().collect(),
            _ => continue,
        };
        for c in children {
            f(c);
            for_each_table(c, f);
        }
    }
}

/// Whether `t` renders a `[header]` of its own (mirrors toml_edit's encoder).
fn has_header(t: &Table) -> bool {
    !t.is_dotted() && (!t.is_implicit() || t.iter().any(|(_, i)| i.is_value()))
}

/// The first table rendered after position `after`.
fn next_table_mut(root: &mut Table, after: isize) -> Option<&mut Table> {
    fn at(t: &mut Table, pos: isize) -> Option<&mut Table> {
        for (_, item) in t.iter_mut() {
            let children: Vec<&mut Table> = match item {
                Item::Table(c) => vec![c],
                Item::ArrayOfTables(aot) => aot.iter_mut().collect(),
                _ => continue,
            };
            for c in children {
                if has_header(c) && c.position() == Some(pos) {
                    return Some(c);
                }
                if let Some(found) = at(c, pos) {
                    return Some(found);
                }
            }
        }
        None
    }
    let mut next: Option<isize> = None;
    for_each_table(root, &mut |t| {
        if let Some(p) = t.position().filter(|&p| p > after && has_header(t)) {
            next = Some(next.map_or(p, |n| n.min(p)));
        }
    });
    at(root, next?)
}

/// toml_edit hangs the comments above a table on that table, so removing it
/// removes them too. Keep the ones a blank line separates from it — those
/// describe the section or the file, not this table — by moving them onto
/// the table that now follows, or to the end of the file.
fn keep_detached_comments(doc: &mut DocumentMut, removed: &Table) {
    let prefix = removed
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or("");
    let lines: Vec<&str> = prefix.split_inclusive('\n').collect();
    let Some(blank) = lines.iter().rposition(|l| l.trim().is_empty()) else {
        return;
    };
    if !lines[..blank]
        .iter()
        .any(|l| l.trim_start().starts_with('#'))
    {
        return;
    }
    let after = removed.position().unwrap_or(isize::MAX);
    match next_table_mut(doc.as_table_mut(), after) {
        Some(next) => {
            // The detached block ends in a blank line; drop the next table's own.
            let own = next
                .decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or("\n");
            let merged = format!(
                "{}{}",
                lines[..=blank].concat(),
                own.trim_start_matches(['\r', '\n'])
            );
            next.decor_mut().set_prefix(merged);
        }
        None => {
            let trailing = doc.trailing().as_str().unwrap_or("");
            let merged = format!("{}{}", lines[..blank].concat(), trailing);
            doc.set_trailing(merged);
        }
    }
}

/// Set `table[key] = new`, keeping the old value's spacing and trailing comment.
/// An equal value is left untouched, so its original spelling survives too.
fn set_value(table: &mut dyn TableLike, key: &str, mut new: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) if same_value(old, &new) => {}
        Some(old) => {
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        None => {
            table.insert(key, Item::Value(new));
        }
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.value() == y.value(),
        (Value::Boolean(x), Value::Boolean(y)) => x.value() == y.value(),
        (Value::Integer(x), Value::Integer(y)) => x.value() == y.value(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| same_value(a, b))
        }
        _ => false,
    }
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
    let range_from = match filter {
        FilterMode::Range(from, _) => from.to_string(),
        _ => String::new(),
    };
    edit_project_config(project_root, |doc| {
        let last = root_table(doc, "last", false)?;
        set_value(last, "version", filter.upper().to_string().into());
        set_value(last, "dev", dev.into());
        set_value(last, "auto", auto.into());
        set_value(last, "mode", filter.name().into());
        set_value(last, "range_from", range_from.into());
        let tags: Array = tags.iter().map(String::as_str).collect();
        set_value(last, "tags", tags.into());
        set_value(last, "profile", profile.unwrap_or("").into());
        set_value(last, "wrap", wrap.unwrap_or("").into());
        set_value(last, "wrap_name", wrap_name.unwrap_or("").into());
        Ok(())
    })
}

pub fn save_include(project_root: &Path, entries: &[IncludeEntry]) -> Result<(), SettingsError> {
    edit_project_config(project_root, |doc| {
        let end = next_position(doc.as_table());
        let mut dropped = Vec::new();
        match doc.get_mut("include") {
            None if entries.is_empty() => {}
            None => {
                let mut aot = ArrayOfTables::new();
                for mut t in reconcile_includes(Vec::new(), entries, Table::new).0 {
                    t.set_position(Some(end));
                    aot.push(t);
                }
                doc.insert("include", Item::ArrayOfTables(aot));
            }
            Some(Item::ArrayOfTables(aot)) => {
                let old: Vec<Table> = aot.iter().cloned().collect();
                let mut prev = old.first().and_then(Table::position).unwrap_or(end);
                let (kept, gone) = reconcile_includes(old, entries, Table::new);
                aot.clear();
                for mut t in kept {
                    // A new table takes its predecessor's position so it renders
                    // right after it rather than wherever key order puts it.
                    match t.position() {
                        Some(p) => prev = p,
                        None => t.set_position(Some(prev)),
                    }
                    aot.push(t);
                }
                if aot.is_empty() {
                    doc.remove("include");
                }
                dropped = gone;
            }
            // `include = [{ from = "..", to = ".." }, ...]`
            Some(Item::Value(Value::Array(arr))) => {
                let old = arr
                    .iter()
                    .map(|v| v.as_inline_table().cloned())
                    .collect::<Option<Vec<InlineTable>>>()
                    .ok_or_else(|| SettingsError("`include` holds a non-table entry".into()))?;
                arr.clear();
                let mut prev_decor: Option<toml_edit::Decor> = None;
                for t in reconcile_includes(old, entries, InlineTable::new).0 {
                    let mut v = Value::InlineTable(t);
                    if v.decor().prefix().is_none() {
                        if let Some(d) = &prev_decor {
                            *v.decor_mut() = d.clone();
                        }
                    }
                    prev_decor = Some(v.decor().clone());
                    arr.push_formatted(v);
                }
            }
            Some(_) => return Err(SettingsError("`include` is not a list of tables".into())),
        }
        // Last first, so comments moving onto the same table stay in file order.
        for t in dropped.iter().rev() {
            keep_detached_comments(doc, t);
        }
        Ok(())
    })
}

/// The persisted include list for `entries`, reusing an existing element for
/// each entry so its comments survive — an exact match first, else one no entry
/// matches any more (the entry `vertion include --remove` trimmed). `fresh`
/// makes the rest. Also returns the old elements left unused.
fn reconcile_includes<T: TableLike + Clone>(
    old: Vec<T>,
    entries: &[IncludeEntry],
    fresh: impl Fn() -> T,
) -> (Vec<T>, Vec<T>) {
    let range_of = |t: &T| {
        let v = |k: &str| {
            t.get(k)
                .and_then(Item::as_str)
                .and_then(|s| parse_version(s).ok())
        };
        Some((v("from")?, v("to")?))
    };
    let mut old: Vec<Option<T>> = old.into_iter().map(Some).collect();
    let mut exact: Vec<Option<T>> = entries
        .iter()
        .map(|e| {
            let i = old.iter().position(|t| {
                t.as_ref()
                    .and_then(range_of)
                    .is_some_and(|(from, to)| from == e.from && to == e.to)
            })?;
            old[i].take()
        })
        .collect();
    let mut leftovers = old.into_iter().flatten();
    let kept = exact
        .iter_mut()
        .zip(entries)
        .map(|(slot, e)| {
            slot.take().unwrap_or_else(|| {
                let mut t = leftovers.next().unwrap_or_else(&fresh);
                // Keep the user's spelling (`"1.1"`) of a bound that didn't move.
                for (key, version) in [("from", &e.from), ("to", &e.to)] {
                    let same = t
                        .get(key)
                        .and_then(Item::as_str)
                        .is_some_and(|s| parse_version(s).ok().as_ref() == Some(version));
                    if !same {
                        set_value(&mut t, key, version.to_string().into());
                    }
                }
                t
            })
        })
        .collect();
    (kept, leftovers.collect())
}

pub fn save_version(project_root: &Path, version: &str) -> Result<(), SettingsError> {
    edit_project_config(project_root, |doc| {
        set_value(
            root_table(doc, "project", false)?,
            "version",
            version.into(),
        );
        Ok(())
    })
}

/// Add/replace (`Some`) or remove (`None`) one condition in the project config.
pub fn save_condition(
    project_root: &Path,
    name: &str,
    def: Option<&ConditionDef>,
) -> Result<(), SettingsError> {
    edit_project_config(project_root, |doc| edit_condition(doc, name, def))
}

fn edit_condition(
    doc: &mut DocumentMut,
    name: &str,
    def: Option<&ConditionDef>,
) -> Result<(), SettingsError> {
    let Some(def) = def else {
        let removed = doc
            .get_mut("conditions")
            .and_then(Item::as_table_like_mut)
            .and_then(|conds| conds.remove(name));
        if let Some(Item::Table(t)) = removed {
            keep_detached_comments(doc, &t);
        }
        return Ok(());
    };
    let inline_parent = doc.get("conditions").is_some_and(Item::is_inline_table);
    let conds = root_table(doc, "conditions", true)?;
    if !conds.contains_key(name) {
        // Match the neighbours: `name = { .. }` beside inline entries,
        // a `[conditions.name]` table otherwise.
        let inline = inline_parent || conds.iter().any(|(_, v)| v.is_inline_table());
        let item = if inline {
            Item::Value(Value::InlineTable(InlineTable::new()))
        } else {
            Item::Table(Table::new())
        };
        conds.insert(name, item);
    }
    let entry = conds
        .get_mut(name)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| SettingsError(format!("condition `{}` is not a table", name)))?;
    let fields = [
        ("global", def.global.as_deref().map(Value::from)),
        ("bool", def.bool.map(Value::from)),
        ("cmd", def.cmd.as_deref().map(Value::from)),
    ];
    for (key, value) in fields {
        match value {
            Some(v) => set_value(entry, key, v),
            None => {
                entry.remove(key);
            }
        }
    }
    Ok(())
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
        fs::write(config_path(&dir), toml::to_string_pretty(&cfg).unwrap()).unwrap();
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

    // ---- writers keep the user's file intact ----

    /// Comments, aligned `=`, a non-alphabetical profile order, a short-form
    /// version and a single-quoted string: everything a full re-serialise loses.
    const HAND_WRITTEN: &str = "\
# Project config, maintained by hand.
# `[last]` is written automatically after every build.

[project]
version    = \"1.2.0\"   # bumped by --auto
input      = \"./src\"
output     = \"./build\"

# Profiles, in the order we care about.
[profiles.zeta]
output = \"./build/zeta\"
tags   = [ \"beta\" ]

[profiles.alpha]
output = './build/alpha'

[[include]]
from = \"1.0\"   # short form on purpose
to   = \"1.1\"

[conditions.fast]
bool = true    # flip for slow builds
";

    const LAST_TABLE: &str = "
[last]
version = \"1.2.0\"
dev = true
auto = false
mode = \"cumulative\"
range_from = \"\"
tags = [\"beta\"]
profile = \"zeta\"
wrap = \"\"
wrap_name = \"\"
";

    fn project_with(name: &str, cfg: &str) -> PathBuf {
        let dir = tmp(name);
        fs::write(config_path(&dir), cfg).unwrap();
        dir
    }

    fn cfg_text(dir: &Path) -> String {
        fs::read_to_string(config_path(dir)).unwrap()
    }

    fn record_last(dir: &Path) {
        let filter = parse_filter(&[String::from("1.2")]).unwrap();
        let tags = [String::from("beta")];
        save_last(dir, &filter, true, false, &tags, Some("zeta"), None, None).unwrap();
    }

    #[test]
    fn save_last_only_appends_the_last_table() {
        let dir = project_with("last-append", HAND_WRITTEN);
        record_last(&dir);
        assert_eq!(cfg_text(&dir), format!("{}{}", HAND_WRITTEN, LAST_TABLE));
        // The result still loads as a config.
        assert_eq!(load_or_default(&dir).unwrap().last.profile, "zeta");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_last_goes_above_a_closing_comment_block() {
        // The block stays where it is and doesn't get attached to `[last]`.
        let closing = "\n# ---- examples ----\n# [conditions.x]\n# bool = true\n";
        let dir = project_with(
            "last-closing-comment",
            &format!("{}{}", HAND_WRITTEN, closing),
        );
        record_last(&dir);
        assert_eq!(
            cfg_text(&dir),
            format!("{}{}{}", HAND_WRITTEN, LAST_TABLE, closing)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_last_keeps_crlf_line_endings() {
        let crlf = HAND_WRITTEN.replace('\n', "\r\n");
        let dir = project_with("last-crlf", &crlf);
        record_last(&dir);
        let expected = format!("{}{}", HAND_WRITTEN, LAST_TABLE).replace('\n', "\r\n");
        assert_eq!(cfg_text(&dir), expected);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_last_updates_an_existing_table_in_place() {
        let before = HAND_WRITTEN.replace(
            "\n# Profiles",
            "\n[last]              # managed by vertion\n\
             version   = \"1.0.0\"\n\
             dev       = false\n\
             mode      = \"cumulative\"\n\
             tags      = []\n\
             \n# Profiles",
        );
        let dir = project_with("last-update", &before);
        record_last(&dir);
        let expected = HAND_WRITTEN.replace(
            "\n# Profiles",
            "\n[last]              # managed by vertion\n\
             version   = \"1.2.0\"\n\
             dev       = true\n\
             mode      = \"cumulative\"\n\
             tags      = [\"beta\"]\n\
             auto = false\n\
             range_from = \"\"\n\
             profile = \"zeta\"\n\
             wrap = \"\"\n\
             wrap_name = \"\"\n\
             \n# Profiles",
        );
        assert_eq!(cfg_text(&dir), expected);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeating_the_same_build_leaves_the_file_alone() {
        let dir = project_with("last-idempotent", HAND_WRITTEN);
        record_last(&dir);
        let once = cfg_text(&dir);
        record_last(&dir);
        assert_eq!(cfg_text(&dir), once);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_version_changes_only_the_value() {
        let dir = project_with("version", HAND_WRITTEN);
        save_version(&dir, "1.3.0").unwrap();
        assert_eq!(
            cfg_text(&dir),
            HAND_WRITTEN.replace(
                "version    = \"1.2.0\"   # bumped",
                "version    = \"1.3.0\"   # bumped"
            )
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_include_keeps_existing_entries_and_their_comments() {
        let dir = project_with("include", HAND_WRITTEN);
        let v = |s: &str| parse_version(s).unwrap();
        let mut entries = load_or_default(&dir).unwrap().include_entries().unwrap();

        // Add: the new entry lands after the existing one, not at the file's end.
        entries.push(IncludeEntry {
            from: v("2.0"),
            to: v("2.0"),
        });
        save_include(&dir, &entries).unwrap();
        let added = HAND_WRITTEN.replace(
            "to   = \"1.1\"\n",
            "to   = \"1.1\"\n\n[[include]]\nfrom = \"2.0.0\"\nto = \"2.0.0\"\n",
        );
        assert_eq!(cfg_text(&dir), added);

        // Trim + delete: the trimmed entry keeps its comment.
        crate::filter::remove_include_entry(&mut entries, &v("1.0"), &v("1.0.5")).unwrap();
        crate::filter::remove_include_entry(&mut entries, &v("2.0"), &v("2.0")).unwrap();
        save_include(&dir, &entries).unwrap();
        assert_eq!(
            cfg_text(&dir),
            HAND_WRITTEN.replace("from = \"1.0\" ", "from = \"1.0.5\" ")
        );

        // Nothing left: the array goes, the rest stays.
        save_include(&dir, &[]).unwrap();
        let text = cfg_text(&dir);
        assert!(!text.contains("[[include]]"), "{text}");
        assert!(
            text.contains("bool = true    # flip for slow builds"),
            "{text}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn edit(text: &str, name: &str, def: Option<&ConditionDef>) -> String {
        edit_text(text, |doc| edit_condition(doc, name, def)).unwrap()
    }

    #[test]
    fn condition_add_set_remove_touch_only_that_condition() {
        let flag = ConditionDef {
            bool: Some(false),
            ..Default::default()
        };
        let added = edit(HAND_WRITTEN, "slow", Some(&flag));
        assert_eq!(
            added,
            format!("{}\n[conditions.slow]\nbool = false\n", HAND_WRITTEN)
        );

        let probe = ConditionDef {
            cmd: Some("exit 0".into()),
            ..Default::default()
        };
        let set = edit(&added, "slow", Some(&probe));
        assert_eq!(
            set,
            format!("{}\n[conditions.slow]\ncmd = \"exit 0\"\n", HAND_WRITTEN)
        );

        assert_eq!(edit(&set, "slow", None), HAND_WRITTEN);
    }

    #[test]
    fn adding_then_removing_a_condition_is_a_no_op() {
        let text = format!("{}\n# closing notes\n", HAND_WRITTEN);
        let flag = ConditionDef {
            bool: Some(false),
            ..Default::default()
        };
        let added = edit(&text, "slow", Some(&flag));
        assert_eq!(edit(&added, "slow", None), text);
    }

    #[test]
    fn removing_a_condition_keeps_the_section_comment_above_it() {
        let text = "\
[project]
version = \"1.0.0\"

# Conditions, see DOCS.md.

# about a
[conditions.a]
bool = true

[conditions.b]
bool = false
";
        assert_eq!(
            edit(text, "a", None),
            "\
[project]
version = \"1.0.0\"

# Conditions, see DOCS.md.

[conditions.b]
bool = false
"
        );
        // With nothing after it, the section comment stays at the end.
        assert_eq!(
            edit(
                "# global conditions\n\n[conditions.a]\nbool = true\n",
                "a",
                None
            ),
            "# global conditions\n"
        );
    }

    #[test]
    fn condition_follows_inline_neighbours() {
        let text = "[conditions]\nfast = { bool = true }  # quick\n";
        let flag = ConditionDef {
            bool: Some(false),
            ..Default::default()
        };
        assert_eq!(
            edit(text, "slow", Some(&flag)),
            "[conditions]\nfast = { bool = true }  # quick\nslow = { bool = false }\n"
        );
    }

    #[test]
    fn condition_into_a_file_without_conditions() {
        let flag = ConditionDef {
            bool: Some(true),
            ..Default::default()
        };
        assert_eq!(
            edit("", "fast", Some(&flag)),
            "[conditions.fast]\nbool = true\n"
        );
        let text = "# global conditions\n";
        assert_eq!(
            edit(text, "fast", Some(&flag)),
            "# global conditions\n[conditions.fast]\nbool = true\n"
        );
    }
}
