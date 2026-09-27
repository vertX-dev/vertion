use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::config::detect_comment_style;
use crate::parser::{detect_marker, Marker, MarkerKind};
use crate::settings::{normalize_path_key, path_covers};
use crate::variants::VARIANT_PREFIX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub file: PathBuf,
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ValidationSummary {
    pub issues: Vec<ValidationIssue>,
    pub warnings: usize,
    pub errors: usize,
    pub files_scanned: usize,
}

impl ValidationSummary {
    pub fn promote_warnings_to_errors(&mut self) {
        for i in &mut self.issues {
            if i.severity == Severity::Warning {
                i.severity = Severity::Error;
            }
        }
        self.errors += self.warnings;
        self.warnings = 0;
    }

    pub fn ok(&self) -> bool {
        self.errors == 0
    }

    /// Record an issue, keeping the per-severity counts in step.
    pub fn push(&mut self, issue: ValidationIssue) {
        match issue.severity {
            Severity::Warning => self.warnings += 1,
            Severity::Error => self.errors += 1,
        }
        self.issues.push(issue);
    }
}

pub fn validate_file(path: &Path) -> io::Result<Vec<ValidationIssue>> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let style = detect_comment_style(ext);
    let text = fs::read_to_string(path)?;
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Ok(validate_lines(path, &lines, style))
}

fn validate_lines(
    path: &Path,
    lines: &[String],
    style: crate::config::CommentStyle,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    // Stack of (version_token, optional_to, open_line). `to == None` for plain blocks.
    let mut stack: Vec<(String, Option<String>, usize)> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        match detect_marker(line, style) {
            MarkerKind::Malformed(reason) => {
                issues.push(ValidationIssue {
                    file: path.to_path_buf(),
                    line: line_no,
                    severity: Severity::Error,
                    message: format!("malformed marker: {}", reason),
                });
            }
            MarkerKind::Versioned(m)
            | MarkerKind::All(m)
            | MarkerKind::Exclude(m)
            | MarkerKind::TagOnly(m) => {
                // Tag-only markers have no version, so they pair on a synthetic
                // `[tags]` label — everything below then works unchanged.
                let m = Marker {
                    version: m.pair_key(),
                    ..m
                };
                let top_matches = stack
                    .last()
                    .map(|(v, t, _)| v == &m.version && t == &m.to)
                    .unwrap_or(false);
                if top_matches {
                    stack.pop();
                } else {
                    // If the top has the same version but a different `to`, it's a mismatched close.
                    if let Some((v, t, open_line)) = stack.last() {
                        if v == &m.version && t != &m.to {
                            issues.push(ValidationIssue {
                                file: path.to_path_buf(),
                                line: line_no,
                                severity: Severity::Error,
                                message: format!(
                                    "mismatched range close: opened at line {} with to=`{}`, closing with to=`{}`",
                                    open_line,
                                    t.as_deref().unwrap_or("<none>"),
                                    m.to.as_deref().unwrap_or("<none>")
                                ),
                            });
                            // Don't push or pop — treat the close as noise.
                            continue;
                        }
                    }
                    // Duplicate-sibling warning: same (version, to) already open higher.
                    if stack.iter().any(|(v, t, _)| v == &m.version && t == &m.to) {
                        let label = match &m.to {
                            Some(to) => format!("{} {}", m.version, to),
                            None => m.version.clone(),
                        };
                        issues.push(ValidationIssue {
                            file: path.to_path_buf(),
                            line: line_no,
                            severity: Severity::Warning,
                            message: format!(
                                "version `{}` is already open higher in the stack; the close marker will pair with the inner block, leaving the outer one unclosed",
                                label
                            ),
                        });
                    }
                    stack.push((m.version, m.to, line_no));
                }
            }
            // Inline range markers are single-line; nothing to pair, nothing to validate
            // here beyond what `detect_marker` already caught (e.g. from >= to).
            MarkerKind::InlineRange(_) => {}
            MarkerKind::None => {}
        }
    }
    for (v, t, line_no) in &stack {
        let label = match t {
            Some(to) => format!("{} {}", v, to),
            None => v.clone(),
        };
        issues.push(ValidationIssue {
            file: path.to_path_buf(),
            line: *line_no,
            severity: Severity::Error,
            message: format!("unclosed version block `{}`", label),
        });
    }
    issues
}

pub fn validate_project(root: &Path, ignore: &[PathBuf]) -> io::Result<ValidationSummary> {
    let mut summary = ValidationSummary::default();
    let ignore_abs: Vec<PathBuf> = ignore.iter().map(|p| absolute(p.as_path())).collect();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let abs = absolute(path);
        if ignore_abs.iter().any(|ig| abs.starts_with(ig)) {
            continue;
        }
        // Skip files we can't read as text.
        let issues = match validate_file(path) {
            Ok(i) => i,
            Err(_) => continue,
        };
        summary.files_scanned += 1;
        for issue in issues {
            summary.push(issue);
        }
    }
    Ok(summary)
}

/// Flag `[[files]]` entries that cover no file.
///
/// An entry whose path matches nothing does nothing, silently — so after a
/// rename, the file (or a whole directory) ships ungated. `paths` are the
/// normalized entry paths in config order; `inputs` are every input directory
/// the config can build from (`[project]` and each profile), and an entry is
/// live if it covers a file in any of them. Paths are compared as the build
/// sees them: a file inside `.vertion.<target>/` counts as `<target>`.
///
/// Issues point at the entry's `[[files]]` header in `config_text`, falling
/// back to line 1 when the headers can't be lined up with the entries (e.g. an
/// inline `files = [...]` array).
pub fn check_file_entries(
    config: &Path,
    config_text: &str,
    paths: &[String],
    inputs: &[PathBuf],
) -> Vec<ValidationIssue> {
    let inputs: Vec<&PathBuf> = inputs.iter().filter(|p| p.is_dir()).collect();
    // No input to compare against: the build would fail on its own, and every
    // entry would otherwise be reported as stale.
    if paths.is_empty() || inputs.is_empty() {
        return Vec::new();
    }

    // (path the build gates on, path as it sits in the source tree)
    let mut files: Vec<(String, String)> = Vec::new();
    for input in &inputs {
        for entry in WalkDir::new(input).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_dir() {
                continue;
            }
            let rel = entry.path().strip_prefix(input).unwrap_or(entry.path());
            files.push((output_key(rel), normalize_path_key(&rel.to_string_lossy())));
        }
    }

    let headers: Vec<usize> = config_text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with("[[files]]"))
        .map(|(i, _)| i + 1)
        .collect();
    let line_of = |i: usize| {
        if headers.len() == paths.len() {
            headers[i]
        } else {
            1
        }
    };

    let mut issues = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if files.iter().any(|(out, _)| path_covers(path, out)) {
            continue;
        }
        // Naming a variant directory by its source path is the likeliest
        // mistake here, and the fix is mechanical, so spell it out.
        let hint = if files.iter().any(|(_, src)| path_covers(path, src)) {
            format!(
                " — variant directories are matched by the path they produce: use `{}`",
                output_key(Path::new(path))
            )
        } else {
            String::new()
        };
        issues.push(ValidationIssue {
            file: config.to_path_buf(),
            line: line_of(i),
            severity: Severity::Warning,
            message: format!(
                "[[files]] path `{}` matches no file, so it gates nothing{}",
                path, hint
            ),
        });
    }
    issues
}

/// The path a source file is built to, as `[[files]]` sees it: each
/// `.vertion.<target>` component becomes `<target>`, and the variant name that
/// follows it is dropped.
fn output_key(rel: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut comps = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    while let Some(c) = comps.next() {
        match c.strip_prefix(VARIANT_PREFIX) {
            Some(target) if !target.is_empty() => {
                out.push(target.to_string());
                comps.next();
            }
            _ => out.push(c),
        }
    }
    out.join("/")
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(name: &str, body: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("vertion-val-{}-{}.js", name, std::process::id()));
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    #[test]
    fn flags_unclosed_block() {
        let p = tmpfile("unclosed", "x\n//version 1.2 *\ninside\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("unclosed")));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn flags_malformed_marker() {
        let p = tmpfile("malformed", "//version notaver *\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("malformed")));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn clean_file_has_no_issues() {
        let p = tmpfile("clean", "x\n//version 1.2 *\nin\n//version 1.2 *\ny\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues.is_empty());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn flags_range_marker_from_ge_to_as_malformed() {
        let p = tmpfile("rangebad", "//version 2.0 1.3 *\nx\n//version 2.0 1.3 *\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("malformed")));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn flags_unclosed_range_block() {
        let p = tmpfile("rangeopen", "//version 1.3 2.0 *\ninside\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("unclosed") && i.message.contains("1.3 2.0")));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn flags_mismatched_range_close() {
        let p = tmpfile("mismatch", "//version 1.3 2.0 *\nin\n//version 1.3 3.0 *\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("mismatched range close")));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn inline_range_does_not_open_a_block() {
        // Inline range markers shouldn't pollute the stack — a single inline range
        // followed by content must not be flagged as unclosed.
        let p = tmpfile("inline", "//version 1.3 2.0\ninside\n");
        let issues = validate_file(&p).unwrap();
        assert!(issues.is_empty());
        let _ = fs::remove_file(&p);
    }

    fn tree(name: &str, files: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("vertion-val-tree-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for f in files {
            let p = root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        root
    }

    fn check(paths: &[&str], inputs: &[PathBuf], text: &str) -> Vec<ValidationIssue> {
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        check_file_entries(Path::new("vertion.cfg"), text, &paths, inputs)
    }

    #[test]
    fn file_entries_that_cover_files_are_live() {
        let src = tree(
            "live",
            &[
                "assets/logo.png",
                "assets/deep/a.png",
                ".vertion.icon.png/2.0.0.png",
                "ui/.vertion.theme/0.0.0/dark.css",
            ],
        );
        let issues = check(
            &[
                "assets/logo.png",   // a file
                "assets",            // a directory, covering files at any depth
                "icon.png",          // a file variant, by the path it produces
                "ui/theme/dark.css", // inside a folder variant
            ],
            std::slice::from_ref(&src),
            "",
        );
        assert!(issues.is_empty(), "{:?}", issues);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn stale_file_entries_are_warned_at_their_header() {
        let src = tree("stale", &["assets/logo.png"]);
        let text = "[project]\nversion = \"1.0\"\n\n\
                    [[files]]\npath = \"assets/logo.png\"\nversion = \"2.0\"\n\n\
                    [[files]]\npath = \"assets/old.png\"\nversion = \"2.0\"\n\n\
                    [[files]]\npath = \"asset\"\nversion = \"2.0\"\n";
        let issues = check(
            &["assets/logo.png", "assets/old.png", "asset"],
            std::slice::from_ref(&src),
            text,
        );
        assert_eq!(issues.len(), 2, "{:?}", issues);
        assert_eq!((issues[0].line, issues[1].line), (8, 12));
        assert!(issues[0].message.contains("`assets/old.png`"));
        // `asset` is a prefix of `assets` but not a directory of its own.
        assert!(issues[1].message.contains("`asset`"));
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn variant_source_path_gets_a_hint() {
        let src = tree(
            "hint",
            &[
                "assets/.vertion.logo.png/2.0.0.png",
                "ui/.vertion.theme/0.0.0/dark.css",
            ],
        );
        let issues = check(
            &["assets/.vertion.logo.png", "ui/.vertion.theme"],
            std::slice::from_ref(&src),
            "",
        );
        assert_eq!(issues.len(), 2);
        assert!(issues[0].message.ends_with("use `assets/logo.png`"));
        // A folder variant: the suggestion is the folder, not a file inside it.
        assert!(issues[1].message.ends_with("use `ui/theme`"));
        // Headers that can't be lined up with entries fall back to line 1.
        assert_eq!(issues[0].line, 1);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn an_entry_live_in_any_input_is_live() {
        let main = tree("in-main", &["a.png"]);
        let alt = tree("in-alt", &["b.png"]);
        let issues = check(&["a.png", "b.png"], &[main.clone(), alt.clone()], "");
        assert!(issues.is_empty(), "{:?}", issues);
        let _ = fs::remove_dir_all(&main);
        let _ = fs::remove_dir_all(&alt);
    }

    #[test]
    fn missing_input_reports_nothing() {
        let gone = std::env::temp_dir().join("vertion-val-no-such-input");
        assert!(check(&["a.png"], &[gone], "").is_empty());
    }

    #[test]
    fn strict_promotion() {
        let mut s = ValidationSummary {
            warnings: 2,
            ..Default::default()
        };
        s.issues.push(ValidationIssue {
            file: PathBuf::from("x"),
            line: 1,
            severity: Severity::Warning,
            message: "w".into(),
        });
        s.promote_warnings_to_errors();
        assert_eq!(s.warnings, 0);
        assert_eq!(s.errors, 2);
        assert_eq!(s.issues[0].severity, Severity::Error);
    }
}
