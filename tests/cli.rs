//! End-to-end tests that run the real `vertion` binary against a real project
//! on disk.
//!
//! The unit tests inside `src/` cover the filtering logic in isolation; nothing
//! there exercises argument parsing, exit codes, config discovery, or the shape
//! of the tree the builder actually writes. That is what these cover.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

/// The fixture every test builds on:
///
/// ```text
/// 1  const base = 1;
/// 2  //version 1.0 *
/// 3  const one = 1;
/// 4  //version 1.0 *
/// 5  //version 2.0 *
/// 6  const two = 2;
/// 7  //version 2.0 *
/// 8  const tail = 4;
/// ```
///
/// A build at 1.0 keeps lines 1, 3 and 8; a build at 2.0 keeps 1, 3, 6 and 8.
/// The trailing line matters: it gives a stripped line somewhere to fall
/// forward to.
const APP_JS: &str = "\
const base = 1;
//version 1.0 *
const one = 1;
//version 1.0 *
//version 2.0 *
const two = 2;
//version 2.0 *
const tail = 4;
";

/// A file that ends inside a block a 1.0 build strips, so a line in that block
/// has no surviving successor at all.
const TAIL_CUT_JS: &str = "\
const a = 1;
//version 2.0 *
const b = 2;
//version 2.0 *
";

const CFG: &str = "\
[project]
version = \"1.0.0\"
input = \"./src\"
output = \"./build\"
ignore = [\"./build\"]

[build]
increment = \"minor\"
";

fn vertion(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("vertion").expect("binary builds");
    cmd.current_dir(dir);
    cmd
}

/// A temp directory holding `vertion.cfg` and `src/app.js`.
fn project() -> TempDir {
    let tmp = TempDir::new().expect("temp dir");
    fs::create_dir(tmp.path().join("src")).expect("src dir");
    fs::write(tmp.path().join("vertion.cfg"), CFG).expect("cfg");
    fs::write(tmp.path().join("src/app.js"), APP_JS).expect("source");
    fs::write(tmp.path().join("src/tailcut.js"), TAIL_CUT_JS).expect("tailcut source");
    tmp
}

fn built(tmp: &TempDir, version: &str) -> String {
    let p = tmp.path().join("build").join(version).join("app.js");
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

// ------------------------------------------------------------------ basics

#[test]
fn version_flag_reports_the_crate_version() {
    let tmp = TempDir::new().unwrap();
    vertion(tmp.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_the_commands() {
    let tmp = TempDir::new().unwrap();
    vertion(tmp.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("build"))
        .stdout(contains("map"))
        .stdout(contains("watch"));
}

#[test]
fn no_arguments_is_an_error() {
    let tmp = TempDir::new().unwrap();
    vertion(tmp.path()).assert().failure();
}

// ------------------------------------------------------------------ build

#[test]
fn build_keeps_the_target_version_and_strips_the_rest() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .success();

    let out = built(&tmp, "1.0.0");
    assert!(out.contains("const base = 1;"), "base code kept: {out:?}");
    assert!(out.contains("const one = 1;"), "1.0 block kept: {out:?}");
    assert!(
        !out.contains("const two = 2;"),
        "2.0 block stripped: {out:?}"
    );
    assert!(!out.contains("//version"), "markers removed: {out:?}");
}

#[test]
fn a_later_build_is_cumulative() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "2.0"])
        .assert()
        .success();

    let out = built(&tmp, "2.0.0");
    assert!(
        out.contains("const one = 1;"),
        "1.0 still included: {out:?}"
    );
    assert!(out.contains("const two = 2;"), "2.0 included: {out:?}");
}

#[test]
fn only_mode_excludes_lower_versions() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "2.0", "ONLY"])
        .assert()
        .success();

    let out = built(&tmp, "2.0.0");
    assert!(out.contains("const base = 1;"), "base code kept: {out:?}");
    assert!(!out.contains("const one = 1;"), "1.0 excluded: {out:?}");
    assert!(out.contains("const two = 2;"), "2.0 kept: {out:?}");
}

#[test]
fn build_without_a_config_fails() {
    let tmp = TempDir::new().unwrap();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .failure();
}

#[test]
fn an_unparsable_version_fails_with_a_message() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "not-a-version"])
        .assert()
        .failure()
        .stderr(contains("error:"));
}

// ------------------------------------------------------------------ inspect

#[test]
fn init_writes_a_config() {
    let tmp = TempDir::new().unwrap();
    vertion(tmp.path()).arg("init").assert().success();
    assert!(tmp.path().join("vertion.cfg").is_file(), "cfg created");
}

#[test]
fn show_reports_the_version_blocks() {
    let tmp = project();
    vertion(tmp.path())
        .args(["show", "src/app.js"])
        .assert()
        .success()
        .stdout(contains("1.0"))
        .stdout(contains("2.0"));
}

// ------------------------------------------------------------------ map

#[test]
fn map_translates_an_output_line_back_to_its_source() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .success();

    // Output line 2 is `const one = 1;`, which is line 3 of the source.
    vertion(tmp.path())
        .args(["map", "build/1.0.0/app.js:2"])
        .assert()
        .success()
        .stdout(contains("app.js:3"));
}

#[test]
fn map_translates_a_source_line_forward() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .success();

    // Source line 3 survives the build as output line 2.
    vertion(tmp.path())
        .args(["map", "src/app.js:3"])
        .assert()
        .success()
        .stdout(contains("app.js:2"));
}

#[test]
fn map_reports_a_line_the_build_stripped() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .success();

    // Line 6 (`const two = 2;`) is not in a 1.0 build at all, so `map` falls
    // forward to the next line that did survive rather than guessing.
    vertion(tmp.path())
        .args(["map", "src/app.js:6"])
        .assert()
        .success()
        .stdout(contains("stripped"));
}

#[test]
fn map_fails_when_nothing_after_a_stripped_line_survives() {
    let tmp = project();
    vertion(tmp.path())
        .args(["build", "-v", "1.0"])
        .assert()
        .success();

    // tailcut.js ends inside its 2.0 block, so line 3 has no successor to fall
    // forward to. That is reported as an error rather than a wrong answer.
    vertion(tmp.path())
        .args(["map", "src/tailcut.js:3"])
        .assert()
        .failure()
        .stderr(contains("stripped"));
}
