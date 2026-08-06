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
use crate::filter::{tag_passes, FilterMode};
use crate::parser::{conditions_pass, process_file, ProcessOptions};
use crate::settings::{normalize_path_key, FileVersionSpec};
use crate::variants::{DEFAULT_STEM, VARIANT_PREFIX};

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
    /// Strip whole-line comments from included output (`--no-comments`).
    pub no_comments: bool,
    /// Whole-file version assignments (normalized rel path → spec) from config.
    /// A file is excluded when its version fails the filter, or its spec is `EXC`.
    pub file_versions: &'a [(String, FileVersionSpec)],
    /// Resolved `(name, value)` condition pairs for `{cond}` marker tags.
    pub conditions: &'a [(String, bool)],
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
    let mut variant_dirs: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&input_abs).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if is_variant_dir_name(path) {
                variant_dirs.push(path.to_path_buf());
            }
            continue;
        }
        if is_ignored(path, &ignore_abs) {
            continue;
        }
        // Files inside a `.vertion.*` directory are resolved as variants below,
        // never copied through the normal path.
        if inside_variant_dir(path, &input_abs) {
            continue;
        }
        let rel = path.strip_prefix(&input_abs).unwrap_or(path).to_path_buf();
        // Whole-file version gate: exclude files whose config-assigned version
        // fails the filter/tags (or is `EXC`). Passing files fall through and copy as-is.
        match file_version_for(&rel, opts.file_versions) {
            Some(FileVersionSpec::Exclude) => continue,
            Some(FileVersionSpec::At {
                version,
                tags,
                conditions,
            }) if !opts.filter.version_matches(version)
                || !tag_passes(tags, opts.tags)
                || !conditions_pass(conditions, opts.conditions) =>
            {
                continue
            }
            _ => {}
        }
        let dest = output_abs.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        jobs.push((path.to_path_buf(), dest, rel));
    }

    // Resolve `.vertion.<target>` directories: pick one winning variant each and
    // emit it under the target name.
    for dir in &variant_dirs {
        if is_ignored(dir, &ignore_abs) || inside_variant_dir(dir, &input_abs) {
            continue; // nested variant dirs are the outer one's business
        }
        let picked = resolve_variant_dir(dir, &input_abs, &opts, &mut result.warnings)?;
        let Some((src, target_rel)) = picked else {
            continue;
        };
        for (file_src, file_rel) in expand_variant_source(&src, &target_rel)? {
            let dest = output_abs.join(&file_rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            jobs.push((file_src, dest, file_rel));
        }
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
        strip_comments: opts.no_comments,
        conditions: opts.conditions,
    };
    let result = process_file(&lines, style, opts.filter, process_opts);

    // Nothing removed (no markers, no comments stripped) → copy verbatim.
    if !result.had_markers && result.stripped == 0 {
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
    for (line_no, name) in &result.unknown_conditions {
        warnings.push(format!(
            "{}:{}: unknown condition `{}` (treated as false)",
            rel.display(),
            line_no,
            name
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

/// The folder a build will write to: `<root>/<version>`, plus a timestamp
/// suffix under `--dev`. Exposed so the caller can predict it before the build
/// (for `VERTION_*` on condition probes) — note that under `--dev` the
/// prediction and the real folder differ once the minute rolls over, so the
/// post-build value must come from [`BuildResult::output`].
pub(crate) fn compute_output_dir(root: &Path, filter: &FilterMode, dev: bool) -> PathBuf {
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

fn file_version_for<'a>(
    rel: &Path,
    map: &'a [(String, FileVersionSpec)],
) -> Option<&'a FileVersionSpec> {
    if map.is_empty() {
        return None;
    }
    let key = normalize_path_key(&rel.to_string_lossy());
    map.iter().find(|(p, _)| *p == key).map(|(_, v)| v)
}

fn is_variant_dir_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with(VARIANT_PREFIX))
        .unwrap_or(false)
}

/// True when any ancestor between `path` and `root` is a variant directory.
fn inside_variant_dir(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    // Skip the final component: the directory itself isn't "inside" one.
    let mut comps: Vec<_> = rel.components().collect();
    comps.pop();
    comps.iter().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.starts_with(VARIANT_PREFIX))
            .unwrap_or(false)
    })
}

/// Pick the winning variant in `dir`, returning `(source path, target rel path)`.
///
/// Highest matching version wins; `.vertion.default.*` is the fallback when
/// nothing matches. Returns `Ok(None)` (with a warning) when there is no winner.
#[allow(clippy::type_complexity)]
fn resolve_variant_dir(
    dir: &Path,
    input_root: &Path,
    opts: &BuildOptions<'_>,
    warnings: &mut Vec<String>,
) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let Some(target_name) = dir_name.strip_prefix(VARIANT_PREFIX) else {
        return Ok(None);
    };
    if target_name.is_empty() {
        return Err(io::Error::other(format!(
            "{}: variant directory has no target name",
            dir.display()
        )));
    }
    let target_ext = Path::new(target_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    // Where the produced file/folder lands, relative to the build root.
    let parent_rel = dir
        .parent()
        .and_then(|p| p.strip_prefix(input_root).ok())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let target_rel = parent_rel.join(target_name);

    let mut best: Option<(crate::variants::VariantSpec, PathBuf)> = None;
    let mut fallback: Option<PathBuf> = None;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_str().unwrap_or_default();
        let is_dir = path.is_dir();

        // A folder variant has no extension to check; a file variant must match
        // the extension declared by the directory name.
        let stem = if is_dir {
            name.to_string()
        } else {
            let ext = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            if ext != target_ext {
                return Err(io::Error::other(format!(
                    "{}: variant `{}` does not match the `{}` extension declared by the directory",
                    dir.display(),
                    name,
                    target_ext.as_deref().unwrap_or("<none>")
                )));
            }
            Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string()
        };

        let spec = crate::variants::parse_variant_stem(&stem)
            .map_err(|e| io::Error::other(format!("{}: {}", path.display(), e)))?;
        if spec.is_default {
            fallback = Some(path);
            continue;
        }
        if !crate::variants::spec_matches(&spec, opts.filter, opts.tags, opts.conditions) {
            continue;
        }
        match &best {
            Some((best_spec, best_path)) => {
                let (a, b) = (spec.rank(), best_spec.rank());
                if a > b {
                    best = Some((spec, path));
                } else if a == b {
                    return Err(io::Error::other(format!(
                        "{}: variants `{}` and `{}` both match at the same version — ambiguous",
                        dir.display(),
                        best_path.file_name().unwrap_or_default().to_string_lossy(),
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )));
                }
            }
            None => best = Some((spec, path)),
        }
    }

    if let Some((_, src)) = best {
        return Ok(Some((src, target_rel)));
    }
    if let Some(src) = fallback {
        return Ok(Some((src, target_rel)));
    }
    warnings.push(format!(
        "{}: no variant matches this build and no `{}` fallback — `{}` will be missing",
        target_rel.display(),
        DEFAULT_STEM,
        target_rel.display()
    ));
    Ok(None)
}

/// A file variant is one job; a folder variant expands to its whole subtree.
fn expand_variant_source(src: &Path, target_rel: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    if src.is_file() {
        return Ok(vec![(src.to_path_buf(), target_rel.to_path_buf())]);
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            continue;
        }
        let inner = p.strip_prefix(src).unwrap_or(p);
        out.push((p.to_path_buf(), target_rel.join(inner)));
    }
    Ok(out)
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
    use crate::filter::{parse_filter, parse_version};
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
            no_comments: false,
            file_versions: &[],
            conditions: &[],
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
    fn predicted_output_dir_matches_what_the_build_writes() {
        // main.rs predicts the output folder *before* the build so condition
        // probes get a populated VERTION_OUTPUT. If the two ever drift, probes
        // silently test the wrong directory — so pin them together here.
        let root = tmpdir("predict");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("a.js"), "keep\n");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let predicted = absolute(&compute_output_dir(&output, &filter, false));
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
            no_comments: false,
            file_versions: &[],
            conditions: &[],
        };
        let result = build_project(opts).unwrap();
        assert_eq!(predicted, result.output);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_version_excludes_and_keeps() {
        let root = tmpdir("filever");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("logo.png"), "binary-ish");
        write_file(&input.join("data.json"), "{}");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        // logo assigned 2.0 (fails <=1.0 → excluded); data assigned 1.0 (passes → kept).
        let file_versions = vec![
            (
                "logo.png".to_string(),
                FileVersionSpec::At {
                    version: parse_version("2.0").unwrap(),
                    tags: vec![],
                    conditions: vec![],
                },
            ),
            (
                "data.json".to_string(),
                FileVersionSpec::At {
                    version: parse_version("1.0").unwrap(),
                    tags: vec![],
                    conditions: vec![],
                },
            ),
        ];
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
            no_comments: false,
            file_versions: &file_versions,
            conditions: &[],
        };
        let result = build_project(opts).unwrap();
        assert!(!result.output.join("logo.png").exists());
        assert!(result.output.join("data.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_version_respects_tag_filter() {
        let root = tmpdir("filetags");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("combat.png"), "x");
        write_file(&input.join("shared.png"), "y");
        let filter = parse_filter(&[String::from("2.0")]).unwrap();
        let file_versions = vec![
            (
                "combat.png".to_string(),
                FileVersionSpec::At {
                    version: parse_version("1.0").unwrap(),
                    tags: vec!["combat".to_string()],
                    conditions: vec![],
                },
            ),
            (
                "shared.png".to_string(),
                FileVersionSpec::At {
                    version: parse_version("1.0").unwrap(),
                    tags: vec![],
                    conditions: vec![],
                },
            ),
        ];
        // Build with --tag inventory: the [combat] file is dropped, the untagged one kept.
        let tags = vec!["inventory".to_string()];
        let opts = BuildOptions {
            input: &input,
            output_root: &output,
            filter: &filter,
            ignore: &[],
            tags: &tags,
            dev: false,
            preserve_context: false,
            strict: false,
            show_progress: false,
            no_comments: false,
            file_versions: &file_versions,
            conditions: &[],
        };
        let result = build_project(opts).unwrap();
        assert!(!result.output.join("combat.png").exists());
        assert!(result.output.join("shared.png").exists());
        let _ = fs::remove_dir_all(&root);
    }

    fn variant_opts<'a>(
        input: &'a Path,
        output: &'a Path,
        filter: &'a FilterMode,
        tags: &'a [String],
    ) -> BuildOptions<'a> {
        BuildOptions {
            input,
            output_root: output,
            filter,
            ignore: &[],
            tags,
            dev: false,
            preserve_context: false,
            strict: false,
            show_progress: false,
            no_comments: false,
            file_versions: &[],
            conditions: &[],
        }
    }

    #[test]
    fn variant_dir_picks_highest_matching_version() {
        let root = tmpdir("variants");
        let input = root.join("src");
        let output = root.join("build");
        let vdir = input.join("assets/.vertion.logo.png");
        write_file(&vdir.join("0.0.0.png"), "base");
        write_file(&vdir.join("2.0.0.png"), "two");
        write_file(&vdir.join("1.2.3e2.0.0.png"), "window");

        let pick = |spec: &str| {
            let filter = parse_filter(&[spec.to_string()]).unwrap();
            let r = build_project(variant_opts(&input, &output, &filter, &[])).unwrap();
            let out = fs::read_to_string(r.output.join("assets/logo.png")).unwrap();
            // The variant directory itself must never reach the output.
            assert!(!r.output.join("assets/.vertion.logo.png").exists());
            out
        };
        assert_eq!(pick("1.0"), "base"); // only 0.0.0 qualifies
        assert_eq!(pick("1.5"), "window"); // 1.2.3 <= 1.5 < 2.0.0 beats 0.0.0
        assert_eq!(pick("2.5"), "two"); // window excluded at its upper bound
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn variant_tags_are_opt_in_and_outrank_by_version() {
        let root = tmpdir("variant-tags");
        let input = root.join("src");
        let output = root.join("build");
        let vdir = input.join(".vertion.logo.png");
        write_file(&vdir.join("0.0.0.png"), "base");
        write_file(&vdir.join("2.0.0-beta.png"), "beta");

        let filter = parse_filter(&[String::from("2.5")]).unwrap();
        // Without the tag active the beta variant is invisible.
        let r = build_project(variant_opts(&input, &output, &filter, &[])).unwrap();
        assert_eq!(
            fs::read_to_string(r.output.join("logo.png")).unwrap(),
            "base"
        );
        // With it, the higher version wins.
        let tags = vec![String::from("beta")];
        let r2 = build_project(variant_opts(&input, &output, &filter, &tags)).unwrap();
        assert_eq!(
            fs::read_to_string(r2.output.join("logo.png")).unwrap(),
            "beta"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn variant_falls_back_to_default_then_warns() {
        let root = tmpdir("variant-default");
        let input = root.join("src");
        let output = root.join("build");
        let vdir = input.join(".vertion.logo.png");
        write_file(&vdir.join("9.0.0.png"), "future");
        write_file(&vdir.join(".vertion.default.png"), "fallback");

        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let r = build_project(variant_opts(&input, &output, &filter, &[])).unwrap();
        assert_eq!(
            fs::read_to_string(r.output.join("logo.png")).unwrap(),
            "fallback"
        );
        assert!(r.warnings.is_empty());

        // Same tree without the fallback: nothing emitted, and a warning says so.
        let root2 = tmpdir("variant-nomatch");
        let input2 = root2.join("src");
        let output2 = root2.join("build");
        write_file(&input2.join(".vertion.logo.png/9.0.0.png"), "future");
        let r2 = build_project(variant_opts(&input2, &output2, &filter, &[])).unwrap();
        assert!(!r2.output.join("logo.png").exists());
        assert!(r2.warnings.iter().any(|w| w.contains("no variant matches")));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&root2);
    }

    #[test]
    fn variant_extension_mismatch_is_an_error() {
        let root = tmpdir("variant-ext");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join(".vertion.logo.png/2.0.0.jpg"), "wrong");
        let filter = parse_filter(&[String::from("2.5")]).unwrap();
        let r = build_project(variant_opts(&input, &output, &filter, &[]));
        assert!(r.is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_variant_copies_the_whole_subtree() {
        let root = tmpdir("variant-folder");
        let input = root.join("src");
        let output = root.join("build");
        let vdir = input.join(".vertion.assets");
        write_file(&vdir.join("0.0.0/a.txt"), "old-a");
        write_file(&vdir.join("2.0.0/a.txt"), "new-a");
        write_file(&vdir.join("2.0.0/nested/b.txt"), "new-b");

        let filter = parse_filter(&[String::from("2.5")]).unwrap();
        let r = build_project(variant_opts(&input, &output, &filter, &[])).unwrap();
        assert_eq!(
            fs::read_to_string(r.output.join("assets/a.txt")).unwrap(),
            "new-a"
        );
        assert_eq!(
            fs::read_to_string(r.output.join("assets/nested/b.txt")).unwrap(),
            "new-b"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_conditions_gate_the_file() {
        let root = tmpdir("filecond");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("gated.png"), "x");
        write_file(&input.join("negated.png"), "y");
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let at = |conds: Vec<crate::parser::MarkerCondition>| FileVersionSpec::At {
            version: parse_version("1.0").unwrap(),
            tags: vec![],
            conditions: conds,
        };
        let c = |name: &str, negated: bool| crate::parser::MarkerCondition {
            name: name.to_string(),
            negated,
        };
        let file_versions = vec![
            ("gated.png".to_string(), at(vec![c("imgs", false)])),
            ("negated.png".to_string(), at(vec![c("imgs", true)])),
        ];
        // imgs = true → gated.png kept, negated.png ({!imgs}) dropped.
        let conditions = [("imgs".to_string(), true)];
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
            no_comments: false,
            file_versions: &file_versions,
            conditions: &conditions,
        };
        let result = build_project(opts).unwrap();
        assert!(result.output.join("gated.png").exists());
        assert!(!result.output.join("negated.png").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_exc_always_excluded() {
        let root = tmpdir("fileexc");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("draft.png"), "wip");
        let filter = parse_filter(&[String::from("9.9")]).unwrap();
        let file_versions = vec![("draft.png".to_string(), FileVersionSpec::Exclude)];
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
            no_comments: false,
            file_versions: &file_versions,
            conditions: &[],
        };
        let result = build_project(opts).unwrap();
        assert!(!result.output.join("draft.png").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_comments_strips_marker_free_file() {
        let root = tmpdir("nocomments");
        let input = root.join("src");
        let output = root.join("build");
        write_file(&input.join("a.js"), "// doc\ncode;\n");
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
            no_comments: true,
            file_versions: &[],
            conditions: &[],
        };
        let result = build_project(opts).unwrap();
        let a = fs::read_to_string(result.output.join("a.js")).unwrap();
        assert_eq!(a, "code;\n");
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
            no_comments: false,
            file_versions: &[],
            conditions: &[],
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
            no_comments: false,
            file_versions: &[],
            conditions: &[],
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
            no_comments: false,
            file_versions: &[],
            conditions: &[],
        };
        assert!(build_project(opts).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
