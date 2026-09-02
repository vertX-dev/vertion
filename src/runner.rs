use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Names of the variables exported to every command Vertion spawns.
///
/// This set is a **public contract**: `unified.cfg` and `wikidata.cfg` expand
/// these in their configured paths so a build can drive both tools without the
/// version being written down twice. Adding a variable is safe; renaming or
/// removing one is a breaking change. See `DOCS.md` — "Build environment".
pub mod var {
    /// Project root — the directory holding `vertion.cfg`.
    pub const ROOT: &str = "VERTION_ROOT";
    /// The versioned build folder that was just written.
    pub const OUTPUT: &str = "VERTION_OUTPUT";
    /// `OUTPUT`'s parent — the configured `output`, resolved.
    pub const OUTPUT_ROOT: &str = "VERTION_OUTPUT_ROOT";
    /// Input directory actually built from (the wrap folder when `--wrap` is live).
    pub const INPUT: &str = "VERTION_INPUT";
    /// Version the build was filtered at, e.g. `2.5.0`.
    pub const VERSION: &str = "VERTION_VERSION";
    /// Leaf folder name — carries the timestamp suffix under `--dev`.
    pub const VERSION_DIR: &str = "VERTION_VERSION_DIR";
    /// Active profile name; empty string when no profile is in use.
    pub const PROFILE: &str = "VERTION_PROFILE";
    /// Filter mode: `cumulative` | `range` | `only` | `include`.
    pub const MODE: &str = "VERTION_MODE";
    /// Active tag filter, comma-joined; empty when none.
    pub const TAGS: &str = "VERTION_TAGS";
    /// `1` under `--dev`, else `0`.
    pub const DEV: &str = "VERTION_DEV";
}

/// The facts a build knows about itself, before they're flattened into
/// environment variables.
#[derive(Debug, Clone, Copy)]
pub struct BuildFacts<'a> {
    pub root: &'a Path,
    pub input: &'a Path,
    /// The versioned output folder. Under `--dev` this is only a prediction
    /// until the build runs — see [`BuildEnv::with_output`].
    pub output: &'a Path,
    pub version: &'a str,
    pub mode: &'a str,
    pub profile: Option<&'a str>,
    pub tags: &'a [String],
    pub dev: bool,
}

/// Environment applied to every spawned command. Empty for commands that aren't
/// part of a build (`validate`, `extract`, …), which simply leaves `VERTION_*`
/// unset for them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildEnv {
    vars: Vec<(String, String)>,
}

impl BuildEnv {
    pub fn new(facts: &BuildFacts<'_>) -> BuildEnv {
        let mut vars = vec![
            (var::ROOT.to_string(), path_string(&absolute(facts.root))),
            (var::INPUT.to_string(), path_string(&absolute(facts.input))),
            (var::VERSION.to_string(), facts.version.to_string()),
            (var::MODE.to_string(), facts.mode.to_string()),
            // Set-but-empty rather than absent, so a consumer reading the
            // environment block (unified, wikidata) can tell "no profile" from
            // "not run by vertion" — `Ok("")` vs `Err(NotPresent)`.
            //
            // Note this does NOT extend to `%VAR%` interpolation inside a `run`
            // line: cmd.exe expands an empty-valued variable exactly like an
            // undefined one, leaving `%VERTION_PROFILE%` literal. Write the
            // profile name out in `run` commands instead of forwarding it.
            (
                var::PROFILE.to_string(),
                facts.profile.unwrap_or("").to_string(),
            ),
            (var::TAGS.to_string(), facts.tags.join(",")),
            (
                var::DEV.to_string(),
                if facts.dev { "1" } else { "0" }.to_string(),
            ),
        ];
        vars.extend(output_vars(facts.output));
        BuildEnv { vars }
    }

    /// Re-point the output-derived variables at the folder a build actually
    /// wrote. Required under `--dev`, where the folder carries a `Local::now()`
    /// timestamp and so can't be predicted before the build runs.
    pub fn with_output(&self, output: &Path) -> BuildEnv {
        let fresh = output_vars(output);
        let mut vars: Vec<(String, String)> = self
            .vars
            .iter()
            .filter(|(k, _)| !fresh.iter().any(|(fresh_key, _)| fresh_key == k))
            .cloned()
            .collect();
        vars.extend(fresh);
        BuildEnv { vars }
    }

    pub fn vars(&self) -> &[(String, String)] {
        &self.vars
    }

    /// Look one variable up by name. Inspection helper — the build itself only
    /// ever hands the whole set to a child process.
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Apply to a command. `envs` sets each name explicitly, so a stale
    /// `VERTION_*` inherited from an outer build is overwritten, never merged.
    fn apply(&self, cmd: &mut Command) {
        cmd.envs(self.vars().iter().map(|(k, v)| (k, v)));
    }
}

/// The three variables derived from the output folder.
fn output_vars(output: &Path) -> Vec<(String, String)> {
    let abs = absolute(output);
    let parent = abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| abs.clone());
    let leaf = abs
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();
    vec![
        (var::OUTPUT.to_string(), path_string(&abs)),
        (var::OUTPUT_ROOT.to_string(), path_string(&parent)),
        (var::VERSION_DIR.to_string(), leaf),
    ]
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Absolute and lexically normalized (`.` dropped, `..` folded).
///
/// Deliberately *not* `canonicalize`: the output folder often doesn't exist yet
/// when the environment is first built, and on Windows canonicalize returns a
/// `\\?\` UNC path that many tools mishandle. These values are a published
/// contract other tools read, so `W:\p\.\build\.` is not good enough — but
/// resolving symlinks isn't wanted either.
fn absolute(p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    normalize(&joined)
}

fn normalize(p: &Path) -> PathBuf {
    let mut components = p.components().peekable();
    // A Windows prefix (`C:`) has to seed the buffer — pushing it like a normal
    // component would not produce a rooted path.
    let mut out = match components.peek() {
        Some(c @ Component::Prefix(_)) => {
            let c = *c;
            components.next();
            PathBuf::from(c.as_os_str())
        }
        _ => PathBuf::new(),
    };
    for component in components {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Pick the right command source: CLI `--run` overrides profile entirely (no merge).
pub fn resolve_run_commands(cli_run: &[String], profile_run: &[String]) -> Vec<String> {
    if !cli_run.is_empty() {
        cli_run.to_vec()
    } else {
        profile_run.to_vec()
    }
}

/// Run commands sequentially in the given working directory, with the build
/// environment applied.
/// Each command's stdout/stderr is inherited so output streams live.
/// Stops on the first non-zero exit and returns an error describing which command failed.
pub fn execute_run_commands(commands: &[String], cwd: &Path, env: &BuildEnv) -> io::Result<()> {
    for cmd in commands {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            continue;
        }
        let status = spawn_shell(trimmed, cwd, env)?;
        if !status.success() {
            let code = status.code().unwrap_or(-1);
            eprintln!("[run] command failed: {} (exit {})", trimmed, code);
            return Err(io::Error::other(format!(
                "post-build command failed (exit {}): {}",
                code, trimmed
            )));
        }
        println!("[run] {} ✓", trimmed);
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_shell(command: &str, cwd: &Path, env: &BuildEnv) -> io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command]).current_dir(cwd);
    env.apply(&mut cmd);
    cmd.status()
}

#[cfg(not(windows))]
fn spawn_shell(command: &str, cwd: &Path, env: &BuildEnv) -> io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", command]).current_dir(cwd);
    env.apply(&mut cmd);
    cmd.status()
}

/// Run a command purely for its exit status (0 → true). Output is captured and
/// discarded so condition probes don't pollute build output.
pub fn shell_test(command: &str, cwd: &Path, env: &BuildEnv) -> io::Result<bool> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(cwd);
    env.apply(&mut cmd);
    Ok(cmd.output()?.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(root: &'a Path, output: &'a Path) -> BuildFacts<'a> {
        BuildFacts {
            root,
            input: Path::new("./src"),
            output,
            version: "2.5.0",
            mode: "cumulative",
            profile: Some("dev"),
            tags: &[],
            dev: false,
        }
    }

    #[test]
    fn cli_overrides_profile() {
        let cli = vec!["a".to_string()];
        let prof = vec!["b".to_string(), "c".to_string()];
        assert_eq!(resolve_run_commands(&cli, &prof), vec!["a".to_string()]);
    }

    #[test]
    fn empty_cli_falls_back_to_profile() {
        let prof = vec!["b".to_string()];
        assert_eq!(resolve_run_commands(&[], &prof), vec!["b".to_string()]);
    }

    #[test]
    fn both_empty_returns_empty() {
        assert!(resolve_run_commands(&[], &[]).is_empty());
    }

    #[test]
    fn build_env_has_every_variable() {
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        for name in [
            var::ROOT,
            var::OUTPUT,
            var::OUTPUT_ROOT,
            var::INPUT,
            var::VERSION,
            var::VERSION_DIR,
            var::PROFILE,
            var::MODE,
            var::TAGS,
            var::DEV,
        ] {
            assert!(env.get(name).is_some(), "{name} missing");
        }
        assert_eq!(env.get(var::VERSION), Some("2.5.0"));
        assert_eq!(env.get(var::VERSION_DIR), Some("2.5.0"));
        assert_eq!(env.get(var::MODE), Some("cumulative"));
        assert_eq!(env.get(var::PROFILE), Some("dev"));
        assert_eq!(env.get(var::DEV), Some("0"));
    }

    #[test]
    fn exported_paths_are_absolute() {
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        for name in [var::ROOT, var::OUTPUT, var::OUTPUT_ROOT, var::INPUT] {
            let value = env.get(name).unwrap();
            assert!(
                Path::new(value).is_absolute(),
                "{name} is relative: {value}"
            );
        }
        assert!(env.get(var::OUTPUT).unwrap().ends_with("2.5.0"));
        assert!(env.get(var::OUTPUT_ROOT).unwrap().ends_with("dev"));
    }

    #[test]
    fn exported_paths_are_normalized() {
        // These values get pasted into other tools' configs — `W:\p\.\build\.`
        // is technically valid and still unacceptable.
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/./dev/2.5.0")));
        for name in [var::ROOT, var::OUTPUT, var::OUTPUT_ROOT, var::INPUT] {
            let value = env.get(name).unwrap();
            assert!(!value.contains("\\.\\"), "{name} keeps a `.`: {value}");
            assert!(!value.contains("/./"), "{name} keeps a `.`: {value}");
            assert!(
                !value.ends_with("\\.") && !value.ends_with("/."),
                "{name} ends in `.`: {value}"
            );
        }
        // `..` folds too.
        assert_eq!(
            normalize(Path::new("/a/b/../c")),
            PathBuf::from("/a").join("c")
        );
        // …and a Windows prefix survives normalization. Only Windows splits
        // `C:\a\.\b` into components; on other platforms it is one ordinary
        // filename, so there is no `.` to fold and the path is not absolute.
        #[cfg(windows)]
        {
            let abs = normalize(Path::new(r"C:\a\.\b"));
            assert_eq!(abs, PathBuf::from(r"C:\a\b"));
            assert!(abs.is_absolute());
        }
    }

    #[test]
    fn absent_profile_is_empty_not_missing() {
        let mut f = facts(Path::new("."), Path::new("./build/2.5.0"));
        f.profile = None;
        let env = BuildEnv::new(&f);
        assert_eq!(env.get(var::PROFILE), Some(""));
    }

    #[test]
    fn an_empty_value_still_reaches_the_child_environment() {
        // The contract consumers rely on: "no profile" (empty) is distinguishable
        // from "not run by vertion" (absent) to anything reading the environment
        // block. Pinned because it's the load-bearing half of the design.
        let cwd = std::env::temp_dir();
        let mut f = facts(Path::new("."), Path::new("./build/2.5.0"));
        f.profile = None;
        let env = BuildEnv::new(&f);
        #[cfg(windows)]
        let (present, absent) = ("set VERTION_PROFILE", "set VERTION_NOT_A_REAL_VAR");
        #[cfg(not(windows))]
        let (present, absent) = (
            "[ -n \"${VERTION_PROFILE+x}\" ]",
            "[ -n \"${VERTION_NOT_A_REAL_VAR+x}\" ]",
        );
        assert!(shell_test(present, &cwd, &env).unwrap());
        assert!(!shell_test(absent, &cwd, &env).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn cmd_does_not_interpolate_an_empty_value() {
        // Executable documentation for a sharp edge: cmd.exe expands an
        // empty-valued variable exactly like an undefined one, so
        // `--profile "%VERTION_PROFILE%"` would pass the literal `%VERTION_PROFILE%`
        // when no profile is active. `run` lines must name the profile outright.
        let cwd = std::env::temp_dir();
        // Exits 1 if cmd expanded the variable (the argument is then ""), 0 if it
        // left `%VERTION_PROFILE%` standing. So `true` means "not interpolated".
        let is_non_empty = "if \"%VERTION_PROFILE%\"==\"\" (exit 1) else (exit 0)";
        let mut no_profile = facts(Path::new("."), Path::new("./build/2.5.0"));
        no_profile.profile = None;
        assert!(shell_test(is_non_empty, &cwd, &BuildEnv::new(&no_profile)).unwrap());

        // A *non-empty* value interpolates normally — the edge is empties only.
        let with_profile = facts(Path::new("."), Path::new("./build/2.5.0"));
        let matches_dev = "if \"%VERTION_PROFILE%\"==\"dev\" (exit 0) else (exit 1)";
        assert!(shell_test(matches_dev, &cwd, &BuildEnv::new(&with_profile)).unwrap());
    }

    #[test]
    fn tags_are_comma_joined() {
        let tags = vec!["beta".to_string(), "inventory".to_string()];
        let mut f = facts(Path::new("."), Path::new("./build/2.5.0"));
        f.tags = &tags;
        assert_eq!(BuildEnv::new(&f).get(var::TAGS), Some("beta,inventory"));
    }

    #[test]
    fn dev_splits_version_from_version_dir() {
        let mut f = facts(
            Path::new("."),
            Path::new("./build/dev/2.5.0_2026-08-02_14-31"),
        );
        f.dev = true;
        let env = BuildEnv::new(&f);
        assert_eq!(env.get(var::VERSION), Some("2.5.0"));
        assert_eq!(env.get(var::VERSION_DIR), Some("2.5.0_2026-08-02_14-31"));
        assert_eq!(env.get(var::DEV), Some("1"));
    }

    #[test]
    fn with_output_repoints_only_output_vars() {
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        let moved = env.with_output(Path::new("./build/dev/2.5.0_2026-08-02_14-31"));
        assert_eq!(moved.get(var::VERSION_DIR), Some("2.5.0_2026-08-02_14-31"));
        assert!(moved.get(var::OUTPUT).unwrap().ends_with("14-31"));
        // Everything else survives untouched.
        assert_eq!(moved.get(var::VERSION), Some("2.5.0"));
        assert_eq!(moved.get(var::PROFILE), Some("dev"));
        assert_eq!(moved.get(var::ROOT), env.get(var::ROOT));
        assert_eq!(moved.vars().len(), env.vars().len());
    }

    #[test]
    fn commands_see_the_variables() {
        let dir = std::env::temp_dir().join(format!("vertion-runner-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        #[cfg(windows)]
        let cmds = vec!["echo %VERTION_VERSION%> seen.txt".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["echo $VERTION_VERSION > seen.txt".to_string()];
        execute_run_commands(&cmds, &dir, &env).unwrap();
        let seen = std::fs::read_to_string(dir.join("seen.txt")).unwrap();
        assert_eq!(seen.trim(), "2.5.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_parent_value_is_overwritten() {
        let dir = std::env::temp_dir().join(format!("vertion-runner-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Safety: single-threaded within this test; the child reads it via the
        // explicit `envs` override, which is exactly what's under test.
        std::env::set_var(var::VERSION, "0.0.0-stale");
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        #[cfg(windows)]
        let cmds = vec!["echo %VERTION_VERSION%> seen.txt".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["echo $VERTION_VERSION > seen.txt".to_string()];
        execute_run_commands(&cmds, &dir, &env).unwrap();
        let seen = std::fs::read_to_string(dir.join("seen.txt")).unwrap();
        std::env::remove_var(var::VERSION);
        assert_eq!(seen.trim(), "2.5.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_test_sees_the_variables() {
        let cwd = std::env::temp_dir();
        let env = BuildEnv::new(&facts(Path::new("."), Path::new("./build/dev/2.5.0")));
        #[cfg(windows)]
        let probe = "if \"%VERTION_VERSION%\"==\"2.5.0\" (exit 0) else (exit 1)";
        #[cfg(not(windows))]
        let probe = "[ \"$VERTION_VERSION\" = \"2.5.0\" ]";
        assert!(shell_test(probe, &cwd, &env).unwrap());
        // …and not when there's no build environment (plain `validate` etc.).
        assert!(!shell_test(probe, &cwd, &BuildEnv::default()).unwrap());
    }

    #[test]
    fn execute_stops_on_first_failure() {
        // First command exits 1, second would succeed — must not run.
        let cwd = std::env::temp_dir();
        #[cfg(windows)]
        let cmds = vec!["exit 1".to_string(), "echo unreachable".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["exit 1".to_string(), "echo unreachable".to_string()];
        let result = execute_run_commands(&cmds, &cwd, &BuildEnv::default());
        assert!(result.is_err());
    }

    #[test]
    fn execute_success_runs_all() {
        let cwd = std::env::temp_dir();
        #[cfg(windows)]
        let cmds = vec!["echo a > NUL".to_string(), "echo b > NUL".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["true".to_string(), "true".to_string()];
        execute_run_commands(&cmds, &cwd, &BuildEnv::default()).unwrap();
    }

    #[test]
    fn runs_in_the_given_cwd() {
        // Backs `--run-here`: a relative-path command must resolve against the
        // cwd we pass, not the process cwd.
        let dir = std::env::temp_dir().join(format!("vertion-runner-cwd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(windows)]
        let cmds = vec!["type nul > marker.txt".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["touch marker.txt".to_string()];
        execute_run_commands(&cmds, &dir, &BuildEnv::default()).unwrap();
        assert!(dir.join("marker.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
