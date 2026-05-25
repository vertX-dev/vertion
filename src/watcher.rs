use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use crate::builder::{build_project, BuildOptions};

pub fn watch_and_rebuild(opts: BuildOptions<'_>) -> std::io::Result<()> {
    // Initial build.
    match build_project(opts.clone()) {
        Ok(r) => println!(
            "vertion: initial build → {} ({} files, {}ms)",
            r.output.display(),
            r.files_processed,
            r.time_ms
        ),
        Err(e) => eprintln!("error: initial build failed: {}", e),
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
                match build_project(opts.clone()) {
                    Ok(r) => println!(
                        "  rebuilt ({} event{}, {} files, {}ms)",
                        n,
                        if n == 1 { "" } else { "s" },
                        r.files_processed,
                        r.time_ms
                    ),
                    Err(e) => eprintln!("  rebuild failed: {}", e),
                }
            }
            Ok(Err(errors)) => {
                eprintln!("  watcher errors: {:?}", errors);
            }
            Err(_) => break, // channel closed
        }
    }
    Ok(())
}
