use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use chrono::Local;
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use crate::builder::{build_project, BuildOptions};
use crate::runner::execute_run_commands;

/// Width of the divider printed before each rebuild.
const DIVIDER_WIDTH: usize = 64;

/// Current wall-clock time as `HH:MM:SS`.
fn now() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

/// A full-width divider with the timestamp embedded on the left, e.g.
/// `──── 14:30:52 ─────────────────────────────────────────────────`.
fn time_divider() -> String {
    let prefix = format!("──── {} ", now());
    let used = prefix.chars().count();
    let dashes = DIVIDER_WIDTH.saturating_sub(used);
    format!("{}{}", prefix, "─".repeat(dashes))
}

pub fn watch_and_rebuild(
    opts: BuildOptions<'_>,
    run_commands: &[String],
    run_here: bool,
) -> std::io::Result<()> {
    // Initial build.
    println!("{}", time_divider());
    match build_project(opts.clone()) {
        Ok(r) => {
            println!(
                "  initial build → {} ({} files, {}ms)",
                r.output.display(),
                r.files_processed,
                r.time_ms
            );
            run_after_build(run_commands, &r.output, run_here);
        }
        Err(e) => eprintln!("  initial build failed: {}", e),
    }

    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)
        .map_err(|e| std::io::Error::other(format!("watcher init failed: {}", e)))?;
    debouncer
        .watcher()
        .watch(opts.input, RecursiveMode::Recursive)
        .map_err(|e| std::io::Error::other(format!("watch failed: {}", e)))?;

    println!(
        "vertion: watching {} (Ctrl+C to exit)",
        opts.input.display()
    );

    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                if events.is_empty() {
                    continue;
                }
                let n = events.len();
                println!("{}", time_divider());
                match build_project(opts.clone()) {
                    Ok(r) => {
                        println!(
                            "  rebuilt ({} event{}, {} files, {}ms)",
                            n,
                            if n == 1 { "" } else { "s" },
                            r.files_processed,
                            r.time_ms
                        );
                        run_after_build(run_commands, &r.output, run_here);
                    }
                    Err(e) => eprintln!("  rebuild failed: {}", e),
                }
            }
            Ok(Err(errors)) => {
                println!("{}", time_divider());
                eprintln!("  watcher errors: {:?}", errors);
            }
            Err(_) => break, // channel closed
        }
    }
    Ok(())
}

/// Run post-build commands after a watch rebuild. Unlike a one-shot `build`, a failing
/// command here must not tear down the watcher — report it and keep watching so the next
/// save gets another chance.
fn run_after_build(commands: &[String], output: &Path, run_here: bool) {
    if commands.is_empty() {
        return;
    }
    let cwd: &Path = if run_here { Path::new(".") } else { output };
    if execute_run_commands(commands, cwd).is_err() {
        eprintln!("  run failed (watching continues)");
    }
}
