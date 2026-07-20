use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::config::detect_comment_style;
use crate::parser::{detect_marker, MarkerKind};

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
            MarkerKind::Versioned(m) | MarkerKind::All(m) | MarkerKind::Exclude(m) => {
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
            match issue.severity {
                Severity::Warning => summary.warnings += 1,
                Severity::Error => summary.errors += 1,
            }
            summary.issues.push(issue);
        }
    }
    Ok(summary)
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
