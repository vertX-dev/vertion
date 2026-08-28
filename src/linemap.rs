//! Tracing a build-output line number back to the source line that produced it.
//!
//! Stripping a version block shifts everything below it, so a stack trace or
//! compiler error pointing into a build tree names a line that doesn't sit at
//! that position in the source. This module reconstructs the correspondence.
//!
//! The map is **recomputed from the source**, never stored: it depends only on
//! the source file and the build parameters recorded in `vertion.manifest.json`,
//! so it adds nothing to build time and can't go stale the way a sidecar file
//! would. The one thing it can't survive is editing the source after the build,
//! which the caller detects by comparing output lengths.

/// One contiguous stretch of lines copied unchanged into the output, as
/// `[out_start, src_start, len]`. Both line numbers are 1-based.
///
/// A file with three stripped blocks has four runs — the encoding stays tiny
/// because stripping removes whole spans, not scattered lines.
pub type Run = [u32; 3];

/// Collapse [`crate::parser::ProcessResult::source_lines`] into runs.
pub fn encode(source_lines: &[u32]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, &src) in source_lines.iter().enumerate() {
        match runs.last_mut() {
            // The output side is contiguous by construction, so only the source
            // side can break a run.
            Some(r) if r[1] + r[2] == src => r[2] += 1,
            _ => runs.push([i as u32 + 1, src, 1]),
        }
    }
    runs
}

/// Total number of output lines the runs describe.
pub fn output_len(runs: &[Run]) -> u32 {
    runs.last().map_or(0, |r| r[0] + r[2] - 1)
}

/// The source line that produced output line `out_line`.
pub fn to_source(runs: &[Run], out_line: u32) -> Option<u32> {
    let i = runs.partition_point(|r| r[0] <= out_line).checked_sub(1)?;
    let r = runs[i];
    (out_line < r[0] + r[2]).then(|| r[1] + (out_line - r[0]))
}

/// The output line holding source line `src_line`, or `None` when that line was
/// stripped from this build.
pub fn to_output(runs: &[Run], src_line: u32) -> Option<u32> {
    let i = runs.partition_point(|r| r[1] <= src_line).checked_sub(1)?;
    let r = runs[i];
    (src_line < r[1] + r[2]).then(|| r[0] + (src_line - r[1]))
}

/// The first surviving source line at or after `src_line`, as `(source, output)`.
/// Lets the caller say something useful about a line that was stripped.
pub fn next_kept(runs: &[Run], src_line: u32) -> Option<(u32, u32)> {
    if let Some(out) = to_output(runs, src_line) {
        return Some((src_line, out));
    }
    let r = runs.iter().find(|r| r[1] > src_line)?;
    Some((r[1], r[0]))
}

// ---- Scanning tool output for file references ----------------------------

/// A `path`/`line` pair spotted in a line of compiler or runtime output.
/// The two spans are byte ranges into the scanned string, so each half can be
/// substituted without disturbing the punctuation around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path_span: (usize, usize),
    pub line_span: (usize, usize),
    pub path: String,
    pub line: u32,
}

/// Bytes that can't be part of a path: whitespace, and the punctuation tools
/// wrap paths in. Deliberately excludes `:` and `\`, which occur inside Windows
/// paths.
const PATH_BREAK: &[u8] = b" \t\"'()[]<>,;=|";

/// Walk backwards from `end` to the start of the path token ending there.
fn path_start(b: &[u8], end: usize) -> usize {
    let mut i = end;
    while i > 0 && !PATH_BREAK.contains(&b[i - 1]) {
        i -= 1;
    }
    i
}

/// Whether a token plausibly names a file. Deliberately permissive — the caller
/// filters for real by resolving it against the build tree, and anything that
/// doesn't resolve is left untouched.
fn looks_like_path(p: &str) -> bool {
    let b = p.as_bytes();
    if p.is_empty() || b[b.len() - 1] == b'/' || b[b.len() - 1] == b'\\' {
        return false;
    }
    // A Windows drive letter is the only colon a path may carry; more than one
    // means we walked back across another `line:col` reference.
    let colons = p.matches(':').count();
    if colons > 1 {
        return false;
    }
    if colons == 1 && !(b.len() >= 3 && b[1] == b':' && b[0].is_ascii_alphabetic()) {
        return false;
    }
    p.contains('/') || p.contains('\\') || p.rfind('.').is_some_and(|i| i > 0 && i + 1 < p.len())
}

/// Find file references in one line of tool output.
///
/// Recognizes the four shapes that cover essentially every toolchain:
/// `path:line` and `path:line:col` (rustc, gcc, node, eslint), `path(line,col)`
/// (tsc, MSVC), and `File "path", line N` (Python).
pub fn scan_refs(line: &str) -> Vec<FileRef> {
    const PY: &str = ", line ";
    let b = line.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;

    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let ds = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let de = i;
        let Ok(num) = line[ds..de].parse::<u32>() else {
            continue;
        };

        // The delimiter right before the digits decides the shape.
        let span = if ds > 0 && matches!(b[ds - 1], b':' | b'(') {
            Some((path_start(b, ds - 1), ds - 1))
        } else if line[..ds].ends_with(PY) {
            // Python: `File "app.py", line 12`
            let head = &line[..ds - PY.len()];
            match (head.rfind('"'), head.rfind("File \"")) {
                (Some(q), Some(f)) if f + 6 <= q => Some((f + 6, q)),
                _ => None,
            }
        } else {
            None
        };

        let Some((ps, pe)) = span else { continue };
        let path = &line[ps..pe];
        if !looks_like_path(path) {
            continue;
        }
        refs.push(FileRef {
            path_span: (ps, pe),
            line_span: (ds, de),
            path: path.to_string(),
            line: num,
        });

        // Step over a trailing `:col` / `,col` so it isn't read as its own reference.
        if matches!(b.get(de), Some(b':') | Some(b',')) {
            let mut j = de + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > de + 1 {
                i = j;
            }
        }
    }
    refs
}

/// Rewrite every reference `resolve` recognizes, leaving the rest of the text
/// byte-for-byte intact. `resolve` returns the replacement `(path, line)`, or
/// `None` to leave that reference alone.
pub fn rewrite<F>(text: &str, mut resolve: F) -> String
where
    F: FnMut(&str, u32) -> Option<(String, u32)>,
{
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.lines().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let mut cursor = 0;
        for r in scan_refs(line) {
            let Some((new_path, new_line)) = resolve(&r.path, r.line) else {
                continue;
            };
            // Spans are disjoint and left-to-right, so one forward pass works.
            out.push_str(&line[cursor..r.path_span.0]);
            out.push_str(&new_path);
            out.push_str(&line[r.path_span.1..r.line_span.0]);
            out.push_str(&new_line.to_string());
            cursor = r.line_span.1;
        }
        out.push_str(&line[cursor..]);
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_contiguous_lines_as_one_run() {
        assert_eq!(encode(&[1, 2, 3, 4]), vec![[1, 1, 4]]);
    }

    #[test]
    fn encodes_a_stripped_block_as_two_runs() {
        // Source lines 3..=6 were stripped.
        assert_eq!(encode(&[1, 2, 7, 8]), vec![[1, 1, 2], [3, 7, 2]]);
    }

    #[test]
    fn encodes_an_empty_output_as_no_runs() {
        assert!(encode(&[]).is_empty());
        assert_eq!(output_len(&[]), 0);
    }

    #[test]
    fn maps_output_lines_back_to_source() {
        let runs = encode(&[1, 2, 7, 8, 20]);
        assert_eq!(to_source(&runs, 1), Some(1));
        assert_eq!(to_source(&runs, 3), Some(7));
        assert_eq!(to_source(&runs, 5), Some(20));
        assert_eq!(to_source(&runs, 6), None);
        assert_eq!(to_source(&runs, 0), None);
    }

    #[test]
    fn maps_source_lines_forward_and_reports_stripped_ones() {
        let runs = encode(&[1, 2, 7, 8, 20]);
        assert_eq!(to_output(&runs, 2), Some(2));
        assert_eq!(to_output(&runs, 8), Some(4));
        // Source line 4 was stripped.
        assert_eq!(to_output(&runs, 4), None);
        assert_eq!(next_kept(&runs, 4), Some((7, 3)));
        assert_eq!(next_kept(&runs, 8), Some((8, 4)));
        // Nothing survives past the last run.
        assert_eq!(next_kept(&runs, 21), None);
    }

    #[test]
    fn round_trips_every_kept_line() {
        let runs = encode(&[3, 4, 5, 11, 12, 40]);
        for out in 1..=output_len(&runs) {
            let src = to_source(&runs, out).unwrap();
            assert_eq!(to_output(&runs, src), Some(out));
        }
    }

    #[test]
    fn scans_rustc_style_references() {
        let r = scan_refs(" --> build/2.0.0/src/game.rs:57:9");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "build/2.0.0/src/game.rs");
        assert_eq!(r[0].line, 57);
    }

    #[test]
    fn does_not_read_a_column_as_a_second_reference() {
        assert_eq!(scan_refs("src/a.rs:57:9").len(), 1);
    }

    #[test]
    fn scans_node_stack_frames_with_windows_paths() {
        let r = scan_refs("    at run (C:\\proj\\build\\2.0.0\\app.js:120:15)");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "C:\\proj\\build\\2.0.0\\app.js");
        assert_eq!(r[0].line, 120);
    }

    #[test]
    fn scans_tsc_paren_references() {
        let r = scan_refs("out/app.ts(42,7): error TS2304");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "out/app.ts");
        assert_eq!(r[0].line, 42);
    }

    #[test]
    fn scans_python_tracebacks() {
        let r = scan_refs("  File \"build/2.0.0/app.py\", line 88, in main");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "build/2.0.0/app.py");
        assert_eq!(r[0].line, 88);
    }

    #[test]
    fn ignores_bare_numbers_and_non_paths() {
        assert!(scan_refs("finished in 1200 ms").is_empty());
        assert!(scan_refs("ratio 3:4 exceeded").is_empty());
    }

    #[test]
    fn rewrite_substitutes_path_and_line_and_keeps_the_rest() {
        let text = "error at build/2.0.0/a.rs:57:9 -- boom\n";
        let got = rewrite(text, |p, l| {
            assert_eq!((p, l), ("build/2.0.0/a.rs", 57));
            Some(("src/a.rs".to_string(), 112))
        });
        assert_eq!(got, "error at src/a.rs:112:9 -- boom\n");
    }

    #[test]
    fn rewrite_keeps_the_paren_shape() {
        let got = rewrite("out/app.ts(42,7): error", |_, _| {
            Some(("src/app.ts".to_string(), 90))
        });
        assert_eq!(got, "src/app.ts(90,7): error");
    }

    #[test]
    fn rewrite_leaves_unresolvable_references_alone() {
        let text = "at /usr/lib/node.js:3:1\n";
        assert_eq!(rewrite(text, |_, _| None), text);
    }

    #[test]
    fn rewrite_handles_two_references_on_one_line() {
        let got = rewrite("a/x.rs:2 and a/y.rs:5", |p, l| {
            Some((p.replace("a/", "s/"), l * 10))
        });
        assert_eq!(got, "s/x.rs:20 and s/y.rs:50");
    }
}
