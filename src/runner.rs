use std::io;
use std::path::Path;
use std::process::Command;

/// Pick the right command source: CLI `--run` overrides profile entirely (no merge).
pub fn resolve_run_commands(cli_run: &[String], profile_run: &[String]) -> Vec<String> {
    if !cli_run.is_empty() {
        cli_run.to_vec()
    } else {
        profile_run.to_vec()
    }
}

/// Run commands sequentially in the given working directory.
/// Each command's stdout/stderr is inherited so output streams live.
/// Stops on the first non-zero exit and returns an error describing which command failed.
pub fn execute_run_commands(commands: &[String], cwd: &Path) -> io::Result<()> {
    for cmd in commands {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            continue;
        }
        let status = spawn_shell(trimmed, cwd)?;
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
fn spawn_shell(command: &str, cwd: &Path) -> io::Result<std::process::ExitStatus> {
    Command::new("cmd")
        .args(["/C", command])
        .current_dir(cwd)
        .status()
}

#[cfg(not(windows))]
fn spawn_shell(command: &str, cwd: &Path) -> io::Result<std::process::ExitStatus> {
    Command::new("sh")
        .args(["-c", command])
        .current_dir(cwd)
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn execute_stops_on_first_failure() {
        // First command exits 1, second would succeed — must not run.
        let cwd = std::env::temp_dir();
        #[cfg(windows)]
        let cmds = vec!["exit 1".to_string(), "echo unreachable".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["exit 1".to_string(), "echo unreachable".to_string()];
        let result = execute_run_commands(&cmds, &cwd);
        assert!(result.is_err());
    }

    #[test]
    fn execute_success_runs_all() {
        let cwd = std::env::temp_dir();
        #[cfg(windows)]
        let cmds = vec!["echo a > NUL".to_string(), "echo b > NUL".to_string()];
        #[cfg(not(windows))]
        let cmds = vec!["true".to_string(), "true".to_string()];
        execute_run_commands(&cmds, &cwd).unwrap();
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
        execute_run_commands(&cmds, &dir).unwrap();
        assert!(dir.join("marker.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
