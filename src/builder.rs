use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Local;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::config::detect_comment_style;
use crate::filter::FilterMode;
use crate::parser::{process_file, ProcessOptions};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BuildResult {
    pub files_processed: usize,
    pub files_modified: usize,
    pub files_copied: usize,
    pub lines_stripped: usize,
    pub time_ms: u128,
    pub output: PathBuf,
    pub version: String,
    pub mode: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BuildOptions<'a> {
    pub input: &'a Path,
    pub output_root: &'a Path,
    pub filter: &'a FilterMode,
    pub ignore: &'a [PathBuf],
    pub tags: &'a [String],
    pub dev: bool,
    pub preserve_context: bool,
    pub strict: bool,
    pub show_progress: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FileOutcome {
    pub processed: bool,
    pub modified: bool,
    pub copied: bool,
    pub lines_stripped: usize,
}

pub fn build_project(opts: BuildOptions<'_>) -> Result<BuildResult, io::Error> {
    if !opts.input.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("input path does not exist: {}", opts.input.display()),
        ));
    }

    let input_abs = absolute(opts.input);
    let output_dir = compute_output_dir(opts.output_root, opts.filter, opts.dev);
    let output_abs = absolute(&output_dir);
    if output_abs.starts_with(&input_abs) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "output path {} must not be inside input path {}",
                output_abs.display(),
                input_abs.display()
            ),
        ));
    }
    if output_abs.exists() && !opts.dev {
        // Plain mode: clear contents to avoid stale carry-over.
        clear_dir(&output_abs)?;
    }
    fs::create_dir_all(&output_abs)?;

    let ignore_abs: Vec<PathBuf> = opts.ignore.iter().map(|p| absolute(p.as_path())).collect();

    let start = Instant::now();
    let mut result = BuildResult {
        output: output_abs.clone(),
        version: opts.filter.upper().to_string(),
        mode: opts.filter.name().to_string(),
        ..Default::default()
    };

    // Pass 1: gather candidate files (so we know the total for the progress bar
    // and can dispatch them across rayon's thread pool).
    let mut jobs: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    for entry in WalkDir::new(&input_abs).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if is_ignored(path, &ignore_abs) {
            continue;
        }
        let rel = path.strip_prefix(&input_abs).unwrap_or(path).to_path_buf();
        let dest = output_abs.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        jobs.push((path.to_path_buf(), dest, rel));
    }

    // Progress bar — auto-hides on non-TTY stderr, so safe to always create.
    let pb = if opts.show_progress {
        let bar = ProgressBar::new(jobs.len() as u64);
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {wide_msg}",
            )
            .unwrap()
            .progress_chars("=>-"),
        );
        bar.enable_steady_tick(Duration::from_millis(120));
        bar
    } else {
        ProgressBar::hidden()
    };

    // Pass 2: process files in parallel. Each closure owns its own `warnings` buffer;
    // we merge them in the sequential aggregation pass below.
    let outcomes: Vec<io::Result<(FileOutcome, Vec<String>)>> = jobs
        .par_iter()
        .map(|(src, dest, rel)| {
            pb.set_message(rel.display().to_string());
            let mut warnings = Vec::new();
            let outcome = build_file(src, dest, &opts, rel, &mut warnings)?;
            pb.inc(1);
            Ok((outcome, warnings))
        })
        .collect();
    pb.finish_and_clear();

    for r in outcomes {
        let (outcome, mut warnings) = r?;
        if outcome.processed {
            result.files_processed += 1;
        }
        if outcome.modified {
            result.files_modified += 1;
        }
        if outcome.copied {
            result.files_copied += 1;
        }
        result.lines_stripped += outcome.lines_stripped;
        result.warnings.append(&mut warnings);
    }

    result.time_ms = start.elapsed().as_millis();
    write_manifest(&output_abs, &result)?;

    if opts.strict && !result.warnings.is_empty() {
        return Err(io::Error::other(format!(
            "strict mode: {} warning(s)",
            result.warnings.len()
        )));
    }

    Ok(result)
}

pub fn build_file(
    src: &Path,
    dest: &Path,
    opts: &BuildOptions<'_>,
    rel: &Path,
    warnings: &mut Vec<String>,
) -> Result<FileOutcome, io::Error> {
    let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("");
    let style = detect_comment_style(ext);

    let content = match fs::read_to_string(src) {
        Ok(c) => c,
        Err(_) => {
            fs::copy(src, dest)?;
            return Ok(FileOutcome {
                processed: false,
                modified: false,
                copied: true,
                lines_stripped: 0,
            });
        }
    };

    let trailing_newline = content.ends_with('\n');
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let process_opts = ProcessOptions {
        tag_filter: opts.tags,
        extract_preserve_context: opts.preserve_context,
    };
    let result = process_file(&lines, style, opts.filter, process_opts);

    if !result.had_markers {
        fs::copy(src, dest)?;
        return Ok(FileOutcome {
            processed: false,
            modified: false,
            copied: true,
            lines_stripped: 0,
        });
    }

    if !result.unclosed.is_empty() {
        warnings.push(format!(
            "{}: unclosed version block(s): {}",
            rel.display(),
            result.unclosed.join(", ")
        ));
    }
    for (line_no, reason) in &result.malformed {
        warnings.push(format!(
            "{}:{}: malformed marker ({})",
            rel.display(),
            line_no,
            reason
        ));
    }

    let mut joined = result.lines.join("\n");
    if trailing_newline && !joined.is_empty() {
        joined.push('\n');
    }
    fs::write(dest, joined)?;

    let modified = result.stripped > 0;
    Ok(FileOutcome {
        processed: true,
        modified,
        copied: false,
        lines_stripped: result.stripped,
    })
}

fn compute_output_dir(root: &Path, filter: &FilterMode, dev: bool) -> PathBuf {
    let version_str = match filter {
        FilterMode::Include(entries) if !entries.is_empty() => {
            let min_from = entries.iter().map(|e| &e.from).min().unwrap();
            let max_to = entries.iter().map(|e| &e.to).max().unwrap();
            format!("{}-{}", min_from, max_to)
        }
        _ => filter.upper().to_string(),
    };
    let suffix = if dev {
        format!("{}_{}", version_str, Local::now().format("%Y-%m-%d_%H-%M"))
    } else {
        version_str
    };
    root.join(suffix)
}

fn write_manifest(output_dir: &Path, result: &BuildResult) -> io::Result<()> {
    let manifest_path = output_dir.join("vertion.manifest.json");
    let json = serde_json::to_string_pretty(result).map_err(io::Error::other)?;
    fs::write(manifest_path, json)
}

fn is_ignored(path: &Path, ignored: &[PathBuf]) -> bool {
    let abs = absolute(path);
    ignored.iter().any(|ig| abs.starts_with(ig))
}

fn clear_dir(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
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
    use crate::filter::parse_filter;
    use std::io::Write;

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vertion-build-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn builds_versioned_subdir_with_manifest() {
        let root = tmpdir("simple");
        let input = root.join("src");
        let output = root.join("build");
        write_file(
            &input.join("a.js"),
            "keep\n//version 2.0 *\nfuture\n//version 2.0 *\ndone\n",
        );
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let opts = BuildOptions {
            input: &input,
            output_root: &output,
            filter: &filter,
            ignore: &[],
            tags: &[],
            dev: false,
            preserve_context: false,
            strict: false,
            show_progress: false,
        };
        let result = build_project(opts).unwrap();
        assert!(result.output.ends_with("1.0.0"));
        let a = fs::read_to_string(result.output.join("a.js")).unwrap();
        assert_eq!(a, "keep\ndone\n");
        let manifest = fs::read_to_string(result.output.join("vertion.manifest.json")).unwrap();
        assert!(manifest.contains("\"files_processed\""));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignore_skips_directories() {
        let root = tmpdir("ignore");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("keep.js"), "x\n");
        write_file(&input.join("node_modules/lib.js"), "ignored\n");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let ignore = vec![input.join("node_modules")];
        let opts = BuildOptions {
            input: &input,
            output_root: &output,
            filter: &filter,
            ignore: &ignore,
            tags: &[],
            dev: false,
            preserve_context: false,
            strict: false,
            show_progress: false,
        };
        let result = build_project(opts).unwrap();
        assert!(result.output.join("keep.js").exists());
        assert!(!result.output.join("node_modules").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dev_uses_timestamped_dir() {
        let root = tmpdir("dev");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("a.js"), "x\n");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let opts = BuildOptions {
            input: &input,
            output_root: &output,
            filter: &filter,
            ignore: &[],
            tags: &[],
            dev: true,
            preserve_context: false,
            strict: false,
            show_progress: false,
        };
        let result = build_project(opts).unwrap();
        let name = result
            .output
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("1.0.0_"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn strict_mode_fails_on_warnings() {
        let root = tmpdir("strict");
        let input = root.join("src");
        let output = root.join("build");
        // Malformed marker => warning
        write_file(&input.join("a.js"), "x\n//version oops *\ny\n");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let opts = BuildOptions {
            input: &input,
            output_root: &output,
            filter: &filter,
            ignore: &[],
            tags: &[],
            dev: false,
            preserve_context: false,
            strict: true,
            show_progress: false,
        };
        assert!(build_project(opts).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
