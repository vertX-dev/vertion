//! Resolving `file:line` references against a build tree.
//!
//! [`linemap`] does the arithmetic; this module supplies the two things it needs
//! from the filesystem — which build a reference belongs to, and which source
//! file produced a given output file — by replaying the build parameters
//! recorded in `vertion.manifest.json`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::builder::{absolute, resolve_variant_dir, BuildOptions, BuildResult, BuildSpec};
use crate::config::detect_comment_style;
use crate::linemap::{self, Run};
use crate::parser::{process_file, ProcessOptions};
use crate::settings::{load_or_default, DEFAULT_CONFIG_NAME};
use crate::variants::VARIANT_PREFIX;

pub const MANIFEST_NAME: &str = "vertion.manifest.json";

/// How far up to walk when looking for a manifest or config.
const MAX_DEPTH: usize = 64;

/// Which way a reference is being translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A build-output line → the source line that produced it.
    ToSource,
    /// A source line → where it landed in the build output.
    ToOutput,
}

/// A translated reference.
#[derive(Debug, Clone)]
pub struct Hit {
    pub direction: Direction,
    /// Path of the reference as given, relative to its own root.
    pub from: PathBuf,
    pub from_line: u32,
    /// Path on the other side, relative to the other root.
    pub to: PathBuf,
    pub to_line: u32,
    /// Set when the answer needed a caveat — the line was stripped, or the
    /// source no longer matches what was built.
    pub note: Option<String>,
}

impl Hit {
    /// Absolute path of the translated side.
    pub fn to_abs(&self, tr: &Trace) -> PathBuf {
        match self.direction {
            Direction::ToSource => tr.input_root.join(&self.to),
            Direction::ToOutput => tr.output_root.join(&self.to),
        }
    }
}

pub struct Trace {
    /// Directory holding the manifest — the build output root.
    pub output_root: PathBuf,
    /// Source tree the build read from.
    pub input_root: PathBuf,
    pub spec: BuildSpec,
    /// Per-file run tables, so a stack trace with many frames in one file
    /// re-parses that file once.
    cache: RefCell<HashMap<PathBuf, Vec<Run>>>,
}

/// The nearest ancestor of `start` (inclusive if it's a directory) holding `name`.
fn find_above(start: &Path, name: &str) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    for _ in 0..MAX_DEPTH {
        if dir.join(name).is_file() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

/// The most recently written build under `output_root`, which is itself a build
/// root in the `--output <dir>` case and a parent of version folders otherwise.
fn latest_build_under(output_root: &Path) -> Option<PathBuf> {
    if output_root.join(MANIFEST_NAME).is_file() {
        return Some(output_root.to_path_buf());
    }
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(output_root).ok()? {
        let dir = entry.ok()?.path();
        let manifest = dir.join(MANIFEST_NAME);
        let Ok(meta) = fs::metadata(&manifest) else {
            continue;
        };
        let Ok(mtime) = meta.modified() else { continue };
        if best.as_ref().map_or(true, |(t, _)| mtime > *t) {
            best = Some((mtime, dir));
        }
    }
    best.map(|(_, d)| d)
}

impl Trace {
    /// Open the build a reference belongs to.
    ///
    /// Looks for a manifest above `hint` first, which is the answer whenever the
    /// hint points into build output. Failing that — the usual case when the
    /// hint is a source file — it finds the project config above `hint` and
    /// takes the most recent build under its configured output root.
    pub fn open(hint: &Path, profile: Option<&str>) -> Result<Trace, String> {
        let hint = absolute(hint);
        let root = match find_above(&hint, MANIFEST_NAME) {
            Some(dir) => dir,
            None => {
                let project = find_above(&hint, DEFAULT_CONFIG_NAME).ok_or_else(|| {
                    format!(
                        "no build found above {} — run this from inside a build \
                         output tree or a project with {}, or pass --build <dir>",
                        hint.display(),
                        DEFAULT_CONFIG_NAME
                    )
                })?;
                let cfg = load_or_default(&project).map_err(|e| e.to_string())?;
                let settings = cfg
                    .resolve_profile(profile)
                    .map_err(|e: crate::settings::SettingsError| e.to_string())?;
                let out = project.join(&settings.output);
                latest_build_under(&out).ok_or_else(|| {
                    format!(
                        "no build found under {} — run `vertion build` first",
                        out.display()
                    )
                })?
            }
        };
        Trace::open_build(&root)
    }

    /// Open a specific build output directory.
    pub fn open_build(root: &Path) -> Result<Trace, String> {
        let path = root.join(MANIFEST_NAME);
        let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let manifest: BuildResult =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {}", path.display(), e))?;
        if manifest.spec.filter.is_none() {
            return Err(format!(
                "{} predates line tracing — re-run the build to record its \
                 settings, then try again",
                path.display()
            ));
        }
        Ok(Trace {
            output_root: absolute(root),
            input_root: absolute(&manifest.spec.input),
            spec: manifest.spec,
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// Which side of the build a path sits on, and its path relative to that root.
    pub fn classify(&self, path: &Path) -> Option<(Direction, PathBuf)> {
        let abs = absolute(path);
        if let Ok(rel) = abs.strip_prefix(&self.output_root) {
            return Some((Direction::ToSource, rel.to_path_buf()));
        }
        if let Ok(rel) = abs.strip_prefix(&self.input_root) {
            return Some((Direction::ToOutput, rel.to_path_buf()));
        }
        // A reference printed by a program whose working directory was one of
        // the roots arrives relative to it, so `absolute()` above guessed wrong.
        let rel = path.strip_prefix(".").unwrap_or(path);
        if self.output_root.join(rel).is_file() {
            return Some((Direction::ToSource, rel.to_path_buf()));
        }
        if self.input_root.join(rel).is_file() {
            return Some((Direction::ToOutput, rel.to_path_buf()));
        }
        None
    }

    /// The `BuildOptions` this build ran with, as far as variant resolution cares.
    fn build_options(&self) -> BuildOptions<'_> {
        let filter = self.spec.filter.as_ref().expect("checked in open_build");
        BuildOptions {
            input: &self.input_root,
            output_root: &self.output_root,
            filter,
            ignore: &[],
            tags: &self.spec.tags,
            dev: false,
            preserve_context: self.spec.preserve_context,
            strict: false,
            show_progress: false,
            no_comments: self.spec.no_comments,
            // Whole-file gates only decide *whether* a file is emitted, never
            // where its lines land, so they don't affect the mapping.
            file_versions: &[],
            conditions: &self.spec.conditions,
            tag_priority: &self.spec.tag_priority,
        }
    }

    /// The source file that produced output path `rel`.
    ///
    /// Usually the same path under the input root, but a `.vertion.<name>/`
    /// variant directory redirects it — for a file variant directly, and for a
    /// folder variant for everything beneath it.
    pub fn source_for(&self, rel: &Path) -> Result<PathBuf, String> {
        let direct = self.input_root.join(rel);
        if direct.is_file() {
            return Ok(direct);
        }
        let opts = self.build_options();
        let mut prefix = rel.to_path_buf();
        let mut tail = PathBuf::new();
        for _ in 0..MAX_DEPTH {
            let (Some(parent), Some(name)) = (prefix.parent(), prefix.file_name()) else {
                break;
            };
            let dir = self.input_root.join(parent).join(format!(
                "{}{}",
                VARIANT_PREFIX,
                name.to_string_lossy()
            ));
            if dir.is_dir() {
                let picked = resolve_variant_dir(&dir, &self.input_root, &opts, &mut Vec::new())
                    .map_err(|e| e.to_string())?;
                if let Some((src, _)) = picked {
                    // A file variant has no tail, and joining an empty path
                    // leaves a trailing separator that no longer names a file.
                    return Ok(if tail.as_os_str().is_empty() {
                        src
                    } else {
                        src.join(&tail)
                    });
                }
            }
            tail = Path::new(name).join(&tail);
            prefix = parent.to_path_buf();
            if prefix.as_os_str().is_empty() {
                break;
            }
        }
        Err(format!("no source file for {}", rel.display()))
    }

    /// Run table for the output file at `rel`, recomputed from its source.
    pub fn runs(&self, rel: &Path) -> Result<Vec<Run>, String> {
        if let Some(cached) = self.cache.borrow().get(rel) {
            return Ok(cached.clone());
        }
        let src = self.source_for(rel)?;
        let content = fs::read_to_string(&src)
            .map_err(|e| format!("{}: {} (not a text file?)", src.display(), e))?;
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("");
        let filter = self.spec.filter.as_ref().expect("checked in open_build");
        let result = process_file(
            &lines,
            detect_comment_style(ext),
            filter,
            ProcessOptions {
                tag_filter: &self.spec.tags,
                extract_preserve_context: self.spec.preserve_context,
                strip_comments: self.spec.no_comments,
                conditions: &self.spec.conditions,
            },
        );
        let runs = linemap::encode(&result.source_lines);
        self.cache
            .borrow_mut()
            .insert(rel.to_path_buf(), runs.clone());
        Ok(runs)
    }

    /// Where an input path lands in the output. The identity, except inside a
    /// `.vertion.<target>/` directory: that is emitted as `<target>`, and the
    /// variant's own name (`2.0.0.png`, or a folder variant's `2.0.0/`) is not
    /// part of the result.
    pub fn output_for(&self, rel: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        let mut comps = rel.components();
        while let Some(c) = comps.next() {
            match c
                .as_os_str()
                .to_string_lossy()
                .strip_prefix(VARIANT_PREFIX)
                .filter(|t| !t.is_empty())
            {
                Some(target) => {
                    out.push(target);
                    comps.next();
                }
                None => out.push(c.as_os_str()),
            }
        }
        out
    }

    /// Whether the built file still has as many lines as the recomputed map
    /// predicts. A mismatch means the source changed after the build, so every
    /// answer below the first edit is suspect.
    fn drifted(&self, rel: &Path, runs: &[Run]) -> bool {
        let Ok(built) = fs::read_to_string(self.output_root.join(rel)) else {
            return false; // Can't check; don't cry wolf.
        };
        built.lines().count() as u32 != linemap::output_len(runs)
    }

    /// Translate one reference, in whichever direction its path implies.
    pub fn resolve(&self, path: &Path, line: u32) -> Result<Hit, String> {
        let (direction, rel) = self.classify(path).ok_or_else(|| {
            format!(
                "{} is in neither {} nor {}",
                path.display(),
                self.output_root.display(),
                self.input_root.display()
            )
        })?;
        // Everything downstream is keyed off the output-relative path — that's
        // what the run table and the drift check are about, and a path inside a
        // variant directory isn't it.
        let out_rel = match direction {
            Direction::ToSource => rel.clone(),
            Direction::ToOutput => self.output_for(&rel),
        };
        if out_rel != rel {
            // Naming a variant only makes sense if it's the one that won; any
            // other answer would be about a file this build never emitted.
            let named = self.input_root.join(&rel);
            if self.source_for(&out_rel).ok().as_deref() != Some(named.as_path()) {
                return Err(format!(
                    "{} is not the variant this build selected for {}",
                    rel.display(),
                    out_rel.display()
                ));
            }
        }

        let runs = self.runs(&out_rel)?;
        let mut note = self
            .drifted(&out_rel, &runs)
            .then(|| "source has changed since this build — mapping may be off".to_string());

        let to_line = match direction {
            Direction::ToSource => linemap::to_source(&runs, line).ok_or_else(|| {
                format!(
                    "{}:{} is past the end of the built file ({} lines)",
                    rel.display(),
                    line,
                    linemap::output_len(&runs)
                )
            })?,
            Direction::ToOutput => match linemap::to_output(&runs, line) {
                Some(l) => l,
                None => {
                    let (src, out) = linemap::next_kept(&runs, line).ok_or_else(|| {
                        format!(
                            "{}:{} was stripped from this build, and nothing after it survived",
                            rel.display(),
                            line
                        )
                    })?;
                    note = Some(format!(
                        "line {} was stripped from this build; showing the next \
                         surviving line ({} in source)",
                        line, src
                    ));
                    out
                }
            },
        };

        // A variant directory means the source path isn't the output path.
        let to = match direction {
            Direction::ToSource => {
                let src = self.source_for(&rel)?;
                src.strip_prefix(&self.input_root)
                    .map(Path::to_path_buf)
                    .unwrap_or(src)
            }
            Direction::ToOutput => out_rel,
        };

        Ok(Hit {
            direction,
            from: rel,
            from_line: line,
            to,
            to_line,
            note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("vertion_trace_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A source tree with one file whose middle block is version-gated.
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let root = tmpdir(name);
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        let marker = format!("{}version", "//");
        fs::write(
            src.join("a.rs"),
            format!(
                "one\ntwo\n{m} 2.0 *\nnew-a\nnew-b\n{m} 2.0 *\nthree\nfour\n",
                m = marker
            ),
        )
        .unwrap();
        (root.clone(), root.join("build"))
    }

    fn build(input: &Path, out_root: &Path, version: &str) -> BuildResult {
        let filter =
            crate::filter::FilterMode::Cumulative(crate::filter::parse_version(version).unwrap());
        crate::builder::build_project(BuildOptions {
            input,
            output_root: out_root,
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
            tag_priority: &[],
        })
        .unwrap()
    }

    #[test]
    fn maps_an_output_line_back_across_a_stripped_block() {
        let (root, out_root) = fixture("back");
        // At 1.0 the 2.0 block is stripped: output is one/two/three/four.
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        let hit = tr.resolve(&result.output.join("a.rs"), 3).unwrap();
        assert_eq!(hit.direction, Direction::ToSource);
        // Output line 3 (`three`) is source line 7.
        assert_eq!(hit.to_line, 7);
        assert!(hit.note.is_none());
    }

    #[test]
    fn maps_a_source_line_forward_into_the_build() {
        let (root, out_root) = fixture("fwd");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        let hit = tr.resolve(&root.join("src").join("a.rs"), 8).unwrap();
        assert_eq!(hit.direction, Direction::ToOutput);
        // Source line 8 (`four`) is output line 4.
        assert_eq!(hit.to_line, 4);
    }

    #[test]
    fn reports_a_source_line_that_this_build_stripped() {
        let (root, out_root) = fixture("stripped");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        // Source line 4 (`new-a`) is inside the 2.0 block.
        let hit = tr.resolve(&root.join("src").join("a.rs"), 4).unwrap();
        assert_eq!(hit.to_line, 3);
        assert!(hit.note.as_deref().unwrap().contains("stripped"));
    }

    #[test]
    fn an_unstripped_build_maps_every_line_to_itself() {
        let (root, out_root) = fixture("identity");
        let result = build(&root.join("src"), &out_root, "2.0");
        let tr = Trace::open_build(&result.output).unwrap();

        // Markers themselves are always removed, so lines above the first one
        // are unshifted and lines below are not.
        let hit = tr.resolve(&result.output.join("a.rs"), 2).unwrap();
        assert_eq!(hit.to_line, 2);
        let hit = tr.resolve(&result.output.join("a.rs"), 3).unwrap();
        assert_eq!(hit.to_line, 4);
    }

    #[test]
    fn rejects_a_path_outside_both_trees() {
        let (root, out_root) = fixture("outside");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        let err = tr
            .resolve(Path::new("/definitely/elsewhere/x.rs"), 1)
            .unwrap_err();
        assert!(err.contains("neither"));
    }

    #[test]
    fn rejects_a_line_past_the_end_of_the_built_file() {
        let (root, out_root) = fixture("past_end");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        let err = tr.resolve(&result.output.join("a.rs"), 99).unwrap_err();
        assert!(err.contains("past the end"));
    }

    #[test]
    fn open_finds_the_build_from_a_path_inside_it() {
        let (root, out_root) = fixture("open_out");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open(&result.output.join("a.rs"), None).unwrap();
        assert_eq!(tr.output_root, absolute(&result.output));
    }

    #[test]
    fn notices_when_the_source_changed_after_the_build() {
        let (root, out_root) = fixture("drift");
        let result = build(&root.join("src"), &out_root, "1.0");
        let tr = Trace::open_build(&result.output).unwrap();

        let file = root.join("src").join("a.rs");
        let text = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("added\n{}", text)).unwrap();

        let hit = tr.resolve(&result.output.join("a.rs"), 1).unwrap();
        assert!(hit.note.as_deref().unwrap().contains("changed"));
    }

    /// A tree where `hud.rs` comes from a `.vertion.hud.rs/` variant directory.
    fn variant_fixture(name: &str) -> PathBuf {
        let root = tmpdir(name);
        let vd = root.join("src").join(".vertion.hud.rs");
        fs::create_dir_all(&vd).unwrap();
        let m = format!("{}version", "//");
        fs::write(
            vd.join("1.0.0.rs"),
            "old_hud();
",
        )
        .unwrap();
        fs::write(
            vd.join("2.0.0.rs"),
            format!(
                "header();
{m} 3.0 *
future();
{m} 3.0 *
new_hud();
",
                m = m
            ),
        )
        .unwrap();
        root
    }

    #[test]
    fn maps_a_variant_sourced_output_line_to_the_variant_it_came_from() {
        let root = variant_fixture("variant_back");
        let result = build(&root.join("src"), &root.join("build"), "2.5");
        let tr = Trace::open_build(&result.output).unwrap();

        // Output is `header(); new_hud();` — line 2 is line 5 of the 2.0.0 variant.
        let hit = tr.resolve(&result.output.join("hud.rs"), 2).unwrap();
        assert_eq!(hit.to, Path::new(".vertion.hud.rs").join("2.0.0.rs"));
        assert_eq!(hit.to_line, 5);
    }

    #[test]
    fn maps_a_variant_source_line_to_the_name_the_build_emitted_it_under() {
        let root = variant_fixture("variant_fwd");
        let result = build(&root.join("src"), &root.join("build"), "2.5");
        let tr = Trace::open_build(&result.output).unwrap();

        let src = root.join("src").join(".vertion.hud.rs").join("2.0.0.rs");
        let hit = tr.resolve(&src, 5).unwrap();
        // Not `.vertion.hud.rs/2.0.0.rs` — the build emitted it as `hud.rs`.
        assert_eq!(hit.to, Path::new("hud.rs"));
        assert_eq!(hit.to_line, 2);
    }

    #[test]
    fn rejects_a_variant_this_build_did_not_select() {
        let root = variant_fixture("variant_loser");
        let result = build(&root.join("src"), &root.join("build"), "2.5");
        let tr = Trace::open_build(&result.output).unwrap();

        let src = root.join("src").join(".vertion.hud.rs").join("1.0.0.rs");
        let err = tr.resolve(&src, 1).unwrap_err();
        assert!(err.contains("not the variant this build selected"));
    }

    #[test]
    fn output_for_leaves_ordinary_paths_alone() {
        let root = variant_fixture("output_for");
        let result = build(&root.join("src"), &root.join("build"), "2.5");
        let tr = Trace::open_build(&result.output).unwrap();

        let plain = Path::new("a").join("b.rs");
        assert_eq!(tr.output_for(&plain), plain);
        // A folder variant drops the variant's own name but keeps what's under it.
        assert_eq!(
            tr.output_for(&Path::new(".vertion.pack").join("2.0.0").join("x.rs")),
            Path::new("pack").join("x.rs")
        );
    }
}
