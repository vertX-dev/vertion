mod builder;
mod config;
mod filter;
mod inspect;
mod parser;
mod runner;
mod settings;
mod stats;
mod validator;
mod watcher;
mod wrap;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use semver::Version;

use crate::builder::{build_project, BuildOptions};
use crate::filter::{
    add_include_entry, autoincrement, parse_filter, parse_include_range, parse_version,
    remove_include_entry, FilterMode, IncludeEntry, IncrementLevel,
};
use crate::settings::{
    load_or_default, save_include, save_last, save_version, write_default_template, VertionConfig,
};

#[derive(Parser, Debug)]
#[command(
    name = "vertion",
    about = "Filter source files by version markers",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build a filtered output tree
    #[command(visible_alias = "b")]
    Build(BuildArgs),
    /// Rebuild using the last build's settings (saved in vertion.toml)
    #[command(visible_alias = "l")]
    Last(BuildArgs),
    /// Show version blocks in a file
    #[command(visible_alias = "s")]
    Show {
        file: PathBuf,
        /// Include tag info in the output
        #[arg(long, short = 'T')]
        tags: bool,
    },
    /// Tree view of version nesting in a file
    #[command(visible_alias = "g")]
    Graph { file: PathBuf },
    /// Validate markers across the project
    #[command(visible_alias = "V")]
    Validate {
        /// Treat warnings as errors
        #[arg(long, short = 'q')]
        strict: bool,
        /// Input directory (defaults to current directory)
        #[arg(long, short = 'I', default_value = ".")]
        input: PathBuf,
        /// Path to ignore (repeatable)
        #[arg(long, short = 'n')]
        ignore: Vec<PathBuf>,
    },
    /// Extract only the tagged/versioned blocks for a version
    #[command(visible_alias = "e")]
    Extract {
        version: String,
        /// Keep surrounding base lines alongside the extracted blocks
        #[arg(long, short = 'c')]
        preserve_context: bool,
        #[arg(long, short = 'I', default_value = ".")]
        input: PathBuf,
        #[arg(long, short = 'o', default_value = "./build")]
        output: PathBuf,
        #[arg(long, short = 'n')]
        ignore: Vec<PathBuf>,
        #[arg(long, short = 't')]
        tag: Vec<String>,
        #[arg(long, short = 'p')]
        profile: Option<String>,
        #[arg(long, short = 'q')]
        strict: bool,
    },
    /// Watch the input directory and rebuild on changes
    #[command(visible_alias = "w")]
    Watch(BuildArgs),
    /// Project-wide marker statistics
    #[command(visible_alias = "S")]
    Stats {
        #[arg(long, short = 'I', default_value = ".")]
        input: PathBuf,
        #[arg(long, short = 'n')]
        ignore: Vec<PathBuf>,
        /// Emit stats as JSON
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Create a vertion.toml in the current directory
    Init,
    /// Manage the persisted [[include]] entries in vertion.toml
    Include(IncludeArgs),
}

#[derive(clap::Args, Debug, Clone)]
struct BuildArgs {
    /// Version spec: `<version>`, `<from> <to>`, or `<version> ONLY`
    #[arg(short = 'v', long = "version-spec", num_args = 1..=2, value_name = "VERSION")]
    version_spec: Vec<String>,

    /// Input directory
    #[arg(long, short = 'I')]
    input: Option<PathBuf>,
    /// Output root directory (a per-version subfolder is created beneath this)
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
    /// Path to ignore (repeatable)
    #[arg(long, short = 'n')]
    ignore: Vec<PathBuf>,
    /// Only include blocks matching this tag (repeatable, OR-logic)
    #[arg(long = "tag", short = 't')]
    tag: Vec<String>,
    /// Use the named profile from vertion.toml
    #[arg(long, short = 'p')]
    profile: Option<String>,
    /// Build to a timestamped folder (does not overwrite)
    #[arg(long, short = 'd')]
    dev: bool,
    /// Treat warnings as errors
    #[arg(long, short = 'q')]
    strict: bool,
    /// Increment config version after a successful build
    #[arg(long, short = 'a')]
    auto: bool,
    /// Increment by major
    #[arg(long, short = 'M')]
    major: bool,
    /// Increment by minor
    #[arg(long, short = 'm')]
    minor: bool,
    /// Increment by patch
    #[arg(long, short = 'P')]
    patch: bool,
    /// Suppress the per-file progress bar (auto-suppressed when stderr is not a TTY)
    #[arg(long)]
    no_progress: bool,
    /// Use the union of all `[[include]]` entries from vertion.toml as the filter
    #[arg(long, short = 'i')]
    include: bool,
    /// Run shell command in the output folder after a successful build (repeatable, sequential)
    #[arg(long, short = 'r')]
    run: Vec<String>,
    /// Wrap project files into an intermediate folder before building.
    /// Forms: `--wrap`, `--wrap perm`, `--wrap temp NAME`, `--wrap perm NAME`.
    /// Default mode is `temp`, default name is `.vertion_wrap`.
    #[arg(long, num_args = 0..=2, value_names = ["MODE", "NAME"])]
    wrap: Option<Vec<String>>,
    /// Allow input paths outside the project root (otherwise a hard error).
    /// Prints a warning when used. Does NOT bypass the output-inside-input check
    /// (use `--wrap` for that).
    #[arg(long)]
    force: bool,
}

#[derive(clap::Args, Debug, Clone)]
struct IncludeArgs {
    /// `<version>` to add, optionally followed by `+ <offset>` for a forward range.
    /// e.g. `include 1.2`, `include 1.2 + 4`. Omit to use `--show` or `--remove`.
    #[arg(num_args = 0..=3)]
    args: Vec<String>,
    /// List all saved [[include]] entries
    #[arg(long, short = 's')]
    show: bool,
    /// Remove or trim an entry: `--remove <from> <to>`
    #[arg(long, short = 'r', num_args = 2, value_names = ["FROM", "TO"])]
    remove: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {}", paint_error("error"), e);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build(args) => cmd_build(args, BuildKind::Build),
        Command::Last(args) => cmd_build(args, BuildKind::Last),
        Command::Show { file, tags } => cmd_show(&file, tags),
        Command::Graph { file } => cmd_graph(&file),
        Command::Validate {
            strict,
            input,
            ignore,
        } => cmd_validate(&input, &ignore, strict),
        Command::Extract {
            version,
            preserve_context,
            input,
            output,
            ignore,
            tag,
            profile,
            strict,
        } => cmd_extract(
            &version,
            preserve_context,
            input,
            output,
            ignore,
            tag,
            profile,
            strict,
        ),
        Command::Watch(args) => cmd_watch(args),
        Command::Stats {
            input,
            ignore,
            json,
        } => cmd_stats(&input, &ignore, json),
        Command::Init => cmd_init(),
        Command::Include(args) => cmd_include(args),
    }
}

enum BuildKind {
    Build,
    Last,
}

fn resolve_filter(
    args: &BuildArgs,
    cfg: &VertionConfig,
    kind: &BuildKind,
) -> Result<FilterMode, String> {
    match kind {
        BuildKind::Build => {
            if args.include {
                let entries = cfg.include_entries().map_err(|e| e.to_string())?;
                if entries.is_empty() {
                    return Err(
                        "--include set but no [[include]] entries found in vertion.toml".into(),
                    );
                }
                return Ok(FilterMode::Include(entries));
            }
            if args.version_spec.is_empty() {
                let v = cfg.project.version.clone();
                parse_filter(&[v]).map_err(|e| e.to_string())
            } else {
                parse_filter(&args.version_spec).map_err(|e| e.to_string())
            }
        }
        BuildKind::Last => {
            if cfg.last.version.is_empty() && cfg.last.mode != "include" {
                return Err("no previous build recorded in vertion.toml".into());
            }
            if cfg.last.mode == "include" {
                let entries = cfg.include_entries().map_err(|e| e.to_string())?;
                if entries.is_empty() {
                    return Err(
                        "last build used --include but no entries remain in vertion.toml".into(),
                    );
                }
                return Ok(FilterMode::Include(entries));
            }
            let mode = cfg.last.mode.as_str();
            match mode {
                "cumulative" | "" => parse_filter(std::slice::from_ref(&cfg.last.version)),
                "range" => parse_filter(&[cfg.last.range_from.clone(), cfg.last.version.clone()]),
                "only" => parse_filter(&[cfg.last.version.clone(), "ONLY".into()]),
                other => Err(crate::filter::FilterError(format!(
                    "unrecognized last.mode `{}`",
                    other
                ))),
            }
            .map_err(|e| e.to_string())
        }
    }
}

fn cmd_build(args: BuildArgs, kind: BuildKind) -> Result<(), String> {
    let project_root = Path::new(".");
    let cfg = load_or_default(project_root).map_err(|e| e.to_string())?;

    // Validate illegal combinations.
    if args.include && !args.version_spec.is_empty() {
        return Err("--include cannot be combined with -v".into());
    }
    if args.include && args.auto {
        return Err("--include cannot be combined with --auto".into());
    }

    let filter = resolve_filter(&args, &cfg, &kind)?;

    if args.auto {
        if matches!(filter, FilterMode::Only(_)) {
            return Err("--auto cannot be combined with ONLY".into());
        }
        if matches!(filter, FilterMode::Include(_)) {
            return Err("--auto cannot be combined with --include".into());
        }
        if matches!(kind, BuildKind::Last) && cfg.last.mode == "only" {
            return Err("--auto cannot be combined with --last when last build used ONLY".into());
        }
    }

    let resolved = cfg
        .resolve_profile(args.profile.as_deref())
        .map_err(|e| e.to_string())?;

    let input = args.input.clone().unwrap_or(resolved.input.clone());
    let output = args.output.clone().unwrap_or(resolved.output.clone());
    let mut ignore = resolved.ignore.clone();
    ignore.extend(args.ignore.iter().cloned());

    let increment = if args.major {
        IncrementLevel::Major
    } else if args.minor {
        IncrementLevel::Minor
    } else if args.patch {
        IncrementLevel::Patch
    } else {
        resolved.increment
    };

    let (tags, dev) = if matches!(kind, BuildKind::Last) {
        let tags = if !args.tag.is_empty() {
            args.tag.clone()
        } else {
            cfg.last.tags.clone()
        };
        let dev = args.dev || cfg.last.dev;
        (tags, dev)
    } else {
        (args.tag.clone(), args.dev)
    };

    // ---- Resolve wrap settings: CLI > profile > [last] (for `vertion last`) ----
    let wrap_settings = resolve_wrap(&args, &resolved, &cfg, &kind)?;

    // ---- Path safety checks before anything writes to disk ----
    path_safety_checks(&input, &output, project_root, args.force, &wrap_settings)?;

    // ---- Optional wrap: copy project files into intermediate folder ----
    let mut build_input = input.clone();
    if let Some((mode, name)) = &wrap_settings {
        let wrap_dir = project_root.join(name);
        let mut wrap_ignored = ignore.clone();
        wrap_ignored.push(absolute(&output));
        wrap_ignored.push(absolute(
            &project_root.join(crate::settings::DEFAULT_CONFIG_NAME),
        ));
        let copied = wrap::wrap_project(&input, &wrap_dir, &wrap_ignored)
            .map_err(|e| format!("wrap failed: {}", e))?;
        println!(
            "wrap ({}): copied {} file(s) → {}",
            mode.as_str(),
            copied,
            wrap_dir.display()
        );
        build_input = wrap_dir;
    }

    let opts = BuildOptions {
        input: &build_input,
        output_root: &output,
        filter: &filter,
        ignore: &ignore,
        tags: &tags,
        dev,
        preserve_context: false,
        strict: args.strict,
        show_progress: !args.no_progress,
    };

    let build_outcome = build_project(opts);

    // Cleanup wrap (temp mode) regardless of build success.
    if let Some((mode, name)) = &wrap_settings {
        if *mode == wrap::WrapMode::Temp {
            let wrap_dir = project_root.join(name);
            if let Err(e) = wrap::cleanup_wrap(&wrap_dir) {
                eprintln!(
                    "{}: failed to clean up wrap dir `{}`: {}",
                    paint_warning("warning"),
                    wrap_dir.display(),
                    e
                );
            }
        }
    }

    let result = build_outcome.map_err(|e| e.to_string())?;

    println!(
        "{} ({})\n  files processed : {}\n  files modified  : {}\n  files copied    : {}\n  lines removed   : {}\n  time            : {}ms\n  output          : {}",
        paint_success("Build completed"),
        result.mode,
        result.files_processed,
        result.files_modified,
        result.files_copied,
        result.lines_stripped,
        result.time_ms,
        result.output.display()
    );
    for w in &result.warnings {
        eprintln!("{}: {}", paint_warning("warning"), w);
    }

    // Post-build commands.
    let run_commands = runner::resolve_run_commands(&args.run, &resolved.run);
    if !run_commands.is_empty() {
        runner::execute_run_commands(&run_commands, &result.output).map_err(|e| e.to_string())?;
    }

    let (wrap_mode_str, wrap_name_str): (Option<&str>, Option<&str>) = match &wrap_settings {
        Some((m, n)) => (Some(m.as_str()), Some(n.as_str())),
        None => (None, None),
    };
    save_last(
        project_root,
        &filter,
        dev,
        args.auto,
        &tags,
        resolved.profile.as_deref(),
        wrap_mode_str,
        wrap_name_str,
    )
    .map_err(|e| e.to_string())?;

    if args.auto {
        let base = filter.upper();
        let next = autoincrement(base, increment);
        save_version(project_root, &next.to_string()).map_err(|e| e.to_string())?;
        println!(
            "auto-increment: vertion.toml [project].version = {} ({})",
            next,
            increment.as_str()
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_extract(
    version_str: &str,
    preserve_context: bool,
    input: PathBuf,
    output: PathBuf,
    ignore: Vec<PathBuf>,
    tag: Vec<String>,
    profile: Option<String>,
    strict: bool,
) -> Result<(), String> {
    let v: Version = parse_version(version_str).map_err(|e| e.to_string())?;
    let filter = FilterMode::Extract(v);
    let project_root = Path::new(".");
    let cfg = load_or_default(project_root).map_err(|e| e.to_string())?;
    let resolved = cfg
        .resolve_profile(profile.as_deref())
        .map_err(|e| e.to_string())?;
    let input = if input.as_os_str() == "." {
        resolved.input.clone()
    } else {
        input
    };
    let output = if output.as_os_str() == "./build" {
        resolved.output.clone()
    } else {
        output
    };
    let mut all_ignore = resolved.ignore.clone();
    all_ignore.extend(ignore);

    let opts = BuildOptions {
        input: &input,
        output_root: &output,
        filter: &filter,
        ignore: &all_ignore,
        tags: &tag,
        dev: false,
        preserve_context,
        strict,
        show_progress: true,
    };
    let result = build_project(opts).map_err(|e| e.to_string())?;
    println!(
        "Extracted version {} ({} files, {} lines stripped, {}ms) → {}",
        result.version,
        result.files_processed,
        result.lines_stripped,
        result.time_ms,
        result.output.display()
    );
    for w in &result.warnings {
        eprintln!("{}: {}", paint_warning("warning"), w);
    }
    Ok(())
}

fn cmd_show(file: &Path, show_tags: bool) -> Result<(), String> {
    let blocks = inspect::collect_blocks_from_file(file).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let total = text.lines().count();
    print!("{}", inspect::render_show(&blocks, show_tags, total));
    Ok(())
}

fn cmd_graph(file: &Path) -> Result<(), String> {
    let blocks = inspect::collect_blocks_from_file(file).map_err(|e| e.to_string())?;
    print!("{}", inspect::render_graph(&blocks));
    Ok(())
}

fn cmd_validate(input: &Path, ignore: &[PathBuf], strict: bool) -> Result<(), String> {
    let mut summary = validator::validate_project(input, ignore).map_err(|e| e.to_string())?;
    if strict {
        summary.promote_warnings_to_errors();
    }
    for issue in &summary.issues {
        let label = match issue.severity {
            validator::Severity::Error => paint_error(issue.severity.label()),
            validator::Severity::Warning => paint_warning(issue.severity.label()),
        };
        eprintln!(
            "{}: {}:{}: {}",
            label,
            issue.file.display(),
            issue.line,
            issue.message
        );
    }
    println!(
        "Validated {} file(s): {} error(s), {} warning(s)",
        summary.files_scanned, summary.errors, summary.warnings
    );
    if !summary.ok() {
        return Err(format!("validation failed: {} error(s)", summary.errors));
    }
    Ok(())
}

fn cmd_watch(args: BuildArgs) -> Result<(), String> {
    let project_root = Path::new(".");
    let cfg = load_or_default(project_root).map_err(|e| e.to_string())?;
    let filter = resolve_filter(&args, &cfg, &BuildKind::Build)?;
    let resolved = cfg
        .resolve_profile(args.profile.as_deref())
        .map_err(|e| e.to_string())?;
    let input = args.input.clone().unwrap_or(resolved.input.clone());
    let output = args.output.clone().unwrap_or(resolved.output.clone());
    let mut ignore = resolved.ignore.clone();
    ignore.extend(args.ignore.iter().cloned());

    let opts = BuildOptions {
        input: &input,
        output_root: &output,
        filter: &filter,
        ignore: &ignore,
        tags: &args.tag,
        dev: args.dev,
        preserve_context: false,
        strict: args.strict,
        show_progress: !args.no_progress,
    };
    watcher::watch_and_rebuild(opts).map_err(|e| e.to_string())
}

fn cmd_stats(input: &Path, ignore: &[PathBuf], json: bool) -> Result<(), String> {
    let summary = stats::gather(input, ignore).map_err(|e| e.to_string())?;
    if json {
        println!("{}", stats::render_json(&summary));
    } else {
        print!("{}", stats::render_table(&summary));
    }
    Ok(())
}

fn cmd_init() -> Result<(), String> {
    let project_root = Path::new(".");
    let path = write_default_template(project_root).map_err(|e| e.to_string())?;
    println!("Created {}", path.display());
    Ok(())
}

fn cmd_include(args: IncludeArgs) -> Result<(), String> {
    let project_root = Path::new(".");
    let cfg = load_or_default(project_root).map_err(|e| e.to_string())?;
    let mut entries = cfg.include_entries().map_err(|e| e.to_string())?;

    // Mutually exclusive operations: show, remove, add.
    if args.show {
        if entries.is_empty() {
            println!("(no [[include]] entries)");
        } else {
            for e in &entries {
                if e.from == e.to {
                    println!("{}", e.from);
                } else {
                    println!("{} → {}", e.from, e.to);
                }
            }
        }
        return Ok(());
    }

    if !args.remove.is_empty() {
        let from = parse_version(&args.remove[0]).map_err(|e| e.to_string())?;
        let to = parse_version(&args.remove[1]).map_err(|e| e.to_string())?;
        remove_include_entry(&mut entries, &from, &to).map_err(|e| e.to_string())?;
        save_include(project_root, &entries).map_err(|e| e.to_string())?;
        println!("Removed include entry {}..{}", from, to);
        return Ok(());
    }

    // Add path: positional args are `<version>` or `<version> + <offset>`.
    if args.args.is_empty() {
        return Err("expected a version (e.g. `include 1.2` or `include 1.2 + 4`)".into());
    }
    let version = &args.args[0];
    let offset = match args.args.len() {
        1 => None,
        3 if args.args[1] == "+" => {
            let n: u64 = args.args[2]
                .parse()
                .map_err(|e| format!("invalid offset `{}`: {}", args.args[2], e))?;
            Some(n)
        }
        _ => {
            return Err(format!(
            "unexpected `include` arguments: {:?} (expected `<version>` or `<version> + <offset>`)",
            args.args
        ))
        }
    };
    let entry: IncludeEntry = parse_include_range(version, offset).map_err(|e| e.to_string())?;
    let added = add_include_entry(&mut entries, entry.clone());
    if !added {
        println!("Entry already exists: {} → {}", entry.from, entry.to);
        return Ok(());
    }
    save_include(project_root, &entries).map_err(|e| e.to_string())?;
    if entry.from == entry.to {
        println!("Added include entry {}", entry.from);
    } else {
        println!("Added include entry {} → {}", entry.from, entry.to);
    }
    Ok(())
}

// ---------- Wrap + path safety helpers ----------

/// Resolve the wrap mode + name from CLI args, profile config, or [last] (for `vertion last`).
/// Returns `None` if wrap is not in use for this build.
fn resolve_wrap(
    args: &BuildArgs,
    resolved: &crate::settings::ResolvedSettings,
    cfg: &VertionConfig,
    kind: &BuildKind,
) -> Result<Option<(wrap::WrapMode, String)>, String> {
    // CLI takes priority. `--wrap` with no args means temp + default name.
    if let Some(parts) = &args.wrap {
        let mode = if parts.is_empty() {
            wrap::WrapMode::Temp
        } else {
            wrap::WrapMode::parse(&parts[0])?
        };
        let name = parts
            .get(1)
            .cloned()
            .unwrap_or_else(|| wrap::DEFAULT_WRAP_NAME.to_string());
        return Ok(Some((mode, name)));
    }
    // For `vertion last`, restore from [last].
    if matches!(kind, BuildKind::Last) && !cfg.last.wrap.is_empty() {
        let mode = wrap::WrapMode::parse(&cfg.last.wrap)?;
        let name = if cfg.last.wrap_name.is_empty() {
            wrap::DEFAULT_WRAP_NAME.to_string()
        } else {
            cfg.last.wrap_name.clone()
        };
        return Ok(Some((mode, name)));
    }
    // Profile config fallback.
    if let Some(mode_str) = &resolved.wrap {
        let mode = wrap::WrapMode::parse(mode_str)?;
        let name = resolved
            .wrap_name
            .clone()
            .unwrap_or_else(|| wrap::DEFAULT_WRAP_NAME.to_string());
        return Ok(Some((mode, name)));
    }
    Ok(None)
}

/// Pre-build path safety checks.
///
/// * Input outside project root → hard error unless `--force` is set (then a warning).
/// * Output inside input → hard error unless `--wrap` is set.
fn path_safety_checks(
    input: &Path,
    output: &Path,
    project_root: &Path,
    force: bool,
    wrap_settings: &Option<(wrap::WrapMode, String)>,
) -> Result<(), String> {
    let input_abs = absolute(input);
    let output_abs = absolute(output);
    let root_abs = absolute(project_root);

    // 1. Input escapes project root.
    if !input_abs.starts_with(&root_abs) {
        if force {
            eprintln!(
                "{}: input `{}` is outside the project root `{}` — proceeding due to --force",
                paint_warning("warning"),
                input_abs.display(),
                root_abs.display()
            );
        } else {
            return Err(format!(
                "input path resolves to '{}' which is outside the project root '{}'.\n       This could result in processing gigabytes of unintended files.\n       Use --force to override.",
                input_abs.display(),
                root_abs.display()
            ));
        }
    }

    // 2. Output inside input. Allowed only when --wrap is active (because the wrap dir
    //    will become the actual build input, sitting outside the output tree).
    if output_abs.starts_with(&input_abs) && wrap_settings.is_none() {
        return Err(
            "output path is inside input path. Use --wrap to isolate project files before building."
                .into(),
        );
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

// ---------- Color helpers ----------

fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // Only colorize when stderr is a real terminal (best-effort on Windows).
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

fn paint(s: &str, code: &str) -> String {
    if color_enabled() {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

fn paint_error(s: &str) -> String {
    paint(s, "31;1")
}
fn paint_warning(s: &str) -> String {
    paint(s, "33;1")
}
fn paint_success(s: &str) -> String {
    paint(s, "32;1")
}
