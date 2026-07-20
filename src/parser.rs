use crate::config::CommentStyle;
use crate::filter::{parse_version, passes, passes_range_marker, FilterMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// First token: a version string like "1.2" or the literal "ALL".
    pub version: String,
    /// Upper bound for range markers (`//version 1.3 2.0`). `None` for single-version markers.
    pub to: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerKind {
    /// A versioned open or close marker; open vs close is decided by the stack at processing time.
    /// May carry a `to` value if it's a range marker block (`//version 1.3 2.0 *`).
    Versioned(Marker),
    /// Explicit `ALL` marker — a block whose contents are always included.
    All(Marker),
    /// Explicit `EXC` marker — a block whose contents are always excluded.
    Exclude(Marker),
    /// Inline range marker (`//version 1.3 2.0` with no `*`) — affects only the next line.
    InlineRange(Marker),
    /// Malformed marker: looks like a marker but couldn't be parsed.
    Malformed(String),
    /// Not a marker.
    None,
}

const KEYWORD: &str = "version";

/// Parse a single line and return its marker classification.
///
/// Marker grammar (after the comment prefix and optional whitespace):
///   `version` <ws> <v1> [<ws> <v2>] [<ws> `[tag1,tag2,...]`] [<ws> `*`] <ws>*
///
/// Where:
/// - `<v1>` is either `ALL` (case-insensitive) or a semver-ish version.
/// - `<v2>` is an optional upper bound for range markers (only valid when `<v1>` is a version).
///   - With `*`: range block (open/close paired by stack).
///   - Without `*`: inline range (applies to the next line only).
pub fn detect_marker(line: &str, style: CommentStyle) -> MarkerKind {
    let trimmed = line.trim_start();
    let prefix = style.prefix();
    let Some(rest) = trimmed.strip_prefix(prefix) else {
        return MarkerKind::None;
    };
    let rest = rest.trim_start();
    let Some(after_kw) = rest.strip_prefix(KEYWORD) else {
        return MarkerKind::None;
    };
    // Require whitespace (or EOL) right after the keyword.
    match after_kw.chars().next() {
        None => {
            return MarkerKind::Malformed("missing version or `ALL` after `version`".into());
        }
        Some(c) if !c.is_whitespace() => return MarkerKind::None,
        _ => {}
    }

    let mut cursor = after_kw.trim_start();
    if cursor.is_empty() {
        return MarkerKind::Malformed("missing version or `ALL` after `version`".into());
    }

    // First token: ALL or a version.
    let token_end = cursor
        .find(|c: char| c.is_whitespace() || c == '[')
        .unwrap_or(cursor.len());
    let token = &cursor[..token_end];
    cursor = cursor[token_end..].trim_start();

    let is_all = token.eq_ignore_ascii_case("ALL");
    let is_exc = token.eq_ignore_ascii_case("EXC");
    let is_keyword = is_all || is_exc;
    if !is_keyword && parse_version(token).is_err() {
        return MarkerKind::Malformed(format!("unparseable version `{}`", token));
    }

    // Optional second version token (range marker upper bound). Only valid for a version.
    let mut to_token: Option<String> = None;
    if !is_keyword && !cursor.is_empty() && !cursor.starts_with('[') && !cursor.starts_with('*') {
        let next_end = cursor
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(cursor.len());
        let next_token = &cursor[..next_end];
        // Peek: only consume as `to` if it parses as a version. Otherwise leave it
        // for the trailing-content check to flag as malformed (preserves existing
        // behavior for things like `// version 2 of foo`).
        if parse_version(next_token).is_ok() {
            to_token = Some(next_token.to_string());
            cursor = cursor[next_end..].trim_start();
        }
    }

    // Optional [tags]
    let mut tags: Vec<String> = Vec::new();
    if let Some(rest_after_bracket) = cursor.strip_prefix('[') {
        let Some(close) = rest_after_bracket.find(']') else {
            return MarkerKind::Malformed("unterminated `[` tag list".into());
        };
        let tag_body = &rest_after_bracket[..close];
        for raw in tag_body.split(',') {
            let t = raw.trim();
            if t.is_empty() {
                return MarkerKind::Malformed("empty tag in list".into());
            }
            tags.push(t.to_string());
        }
        cursor = rest_after_bracket[close + 1..].trim_start();
    }

    // Optional `*` plus only whitespace afterwards.
    let has_star = cursor.starts_with('*');
    if has_star {
        cursor = cursor[1..].trim_start();
    }
    if !cursor.is_empty() {
        return MarkerKind::Malformed(format!("unexpected trailing content `{}`", cursor.trim()));
    }

    // Validate range marker semantics (from < to).
    if let Some(ref to_t) = to_token {
        let from_v = parse_version(token).ok();
        let to_v = parse_version(to_t).ok();
        if let (Some(f), Some(t)) = (from_v, to_v) {
            if f >= t {
                return MarkerKind::Malformed(format!(
                    "range marker has from >= to ({} >= {})",
                    token, to_t
                ));
            }
        }
    }

    let marker = Marker {
        version: token.to_string(),
        to: to_token,
        tags,
    };
    if is_all {
        MarkerKind::All(marker)
    } else if is_exc {
        MarkerKind::Exclude(marker)
    } else if marker.to.is_some() && !has_star {
        MarkerKind::InlineRange(marker)
    } else {
        MarkerKind::Versioned(marker)
    }
}

#[derive(Debug, Default)]
pub struct ProcessResult {
    pub lines: Vec<String>,
    pub had_markers: bool,
    pub unclosed: Vec<String>,
    pub stripped: usize,
    pub malformed: Vec<(usize, String)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessOptions<'a> {
    pub tag_filter: &'a [String],
    pub extract_preserve_context: bool,
    /// Strip whole-line comments (non-marker) from included output.
    pub strip_comments: bool,
}

/// A whole-line comment: first non-whitespace char begins the comment prefix.
/// ponytail: whole-line only — trailing/inline comments and `//` inside string
/// literals are left alone (stripping them needs a real lexer and would corrupt code).
fn is_line_comment(line: &str, style: CommentStyle) -> bool {
    line.trim_start().starts_with(style.prefix())
}

pub fn process_file(
    lines: &[String],
    style: CommentStyle,
    filter: &FilterMode,
    opts: ProcessOptions<'_>,
) -> ProcessResult {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut stack: Vec<Marker> = Vec::new();
    let mut had_markers = false;
    let mut stripped = 0usize;
    let mut malformed: Vec<(usize, String)> = Vec::new();
    let extract = matches!(filter, FilterMode::Extract(_));
    // Inline range pending: forces the next *content* line's inclusion to this value.
    let mut inline_pending: Option<bool> = None;

    for (idx, line) in lines.iter().enumerate() {
        match detect_marker(line, style) {
            MarkerKind::Versioned(m) => {
                had_markers = true;
                stripped += 1;
                // Top-of-stack version+to match closes the block.
                if stack
                    .last()
                    .map(|t| t.version == m.version && t.to == m.to)
                    .unwrap_or(false)
                {
                    stack.pop();
                } else {
                    stack.push(m);
                }
            }
            MarkerKind::All(m) => {
                had_markers = true;
                stripped += 1;
                if stack
                    .last()
                    .map(|t| t.version.eq_ignore_ascii_case("ALL"))
                    .unwrap_or(false)
                {
                    stack.pop();
                } else {
                    stack.push(m);
                }
            }
            MarkerKind::Exclude(m) => {
                had_markers = true;
                stripped += 1;
                if stack
                    .last()
                    .map(|t| t.version.eq_ignore_ascii_case("EXC"))
                    .unwrap_or(false)
                {
                    stack.pop();
                } else {
                    stack.push(m);
                }
            }
            MarkerKind::InlineRange(m) => {
                had_markers = true;
                stripped += 1;
                // Combined: all ancestors pass AND this range marker passes.
                let ancestors_pass = stack
                    .iter()
                    .all(|a| marker_passes_filter(a, filter, opts.tag_filter, extract));
                let self_pass = passes_range_marker(
                    &m.version,
                    m.to.as_deref().unwrap_or(""),
                    &m.tags,
                    filter,
                    opts.tag_filter,
                );
                // In Extract mode the inline range doesn't preserve base — only context preserves.
                let final_pass = if extract {
                    if !opts.extract_preserve_context {
                        // Extract aims at exact-version blocks; ranges aren't tagged blocks,
                        // so they're not emitted unless preserving context.
                        false
                    } else {
                        ancestors_pass && self_pass
                    }
                } else {
                    ancestors_pass && self_pass
                };
                inline_pending = Some(final_pass);
            }
            MarkerKind::Malformed(reason) => {
                had_markers = true;
                stripped += 1;
                malformed.push((idx + 1, reason));
            }
            MarkerKind::None => {
                let include = if let Some(forced) = inline_pending.take() {
                    forced
                } else {
                    decide_inclusion(&stack, filter, opts, extract)
                };
                if include && opts.strip_comments && is_line_comment(line, style) {
                    stripped += 1;
                } else if include {
                    out.push(line.clone());
                } else {
                    stripped += 1;
                }
            }
        }
    }

    let unclosed = stack
        .into_iter()
        .map(|m| match m.to {
            Some(t) => format!("{} {}", m.version, t),
            None => m.version,
        })
        .collect();
    ProcessResult {
        lines: out,
        had_markers,
        unclosed,
        stripped,
        malformed,
    }
}

/// Whether a marker on the ancestor stack passes the active filter.
/// Handles plain versioned, ALL, and range-block markers uniformly.
fn marker_passes_filter(
    m: &Marker,
    filter: &FilterMode,
    tag_filter: &[String],
    extract: bool,
) -> bool {
    if extract {
        // In extract mode the ancestor rule is evaluated separately by decide_inclusion;
        // this helper is only used for inline range eval, where extract is handled
        // by the caller. Default to ancestor passing for non-extract code paths.
        return passes_marker_general(m, filter, tag_filter);
    }
    passes_marker_general(m, filter, tag_filter)
}

fn passes_marker_general(m: &Marker, filter: &FilterMode, tag_filter: &[String]) -> bool {
    if let Some(to) = &m.to {
        passes_range_marker(&m.version, to, &m.tags, filter, tag_filter)
    } else {
        passes(&m.version, &m.tags, filter, tag_filter)
    }
}

fn decide_inclusion(
    stack: &[Marker],
    filter: &FilterMode,
    opts: ProcessOptions<'_>,
    extract: bool,
) -> bool {
    if stack.is_empty() {
        // Base line.
        return if extract {
            opts.extract_preserve_context
        } else {
            true
        };
    }
    if extract {
        // Extract mode: any non-ALL, non-range ancestor matching the target version wins.
        let any_match = stack.iter().any(|m| {
            !m.version.eq_ignore_ascii_case("ALL")
                && m.to.is_none()
                && passes(&m.version, &m.tags, filter, opts.tag_filter)
        });
        if any_match {
            return true;
        }
        // ALL-only (or range-only) ancestor chain → only preserve_context emits.
        let only_passthrough = stack
            .iter()
            .all(|m| m.version.eq_ignore_ascii_case("ALL") || m.to.is_some());
        return only_passthrough && opts.extract_preserve_context;
    }
    // Cumulative / Range / Only / Include: every ancestor must independently pass.
    stack
        .iter()
        .all(|m| passes_marker_general(m, filter, opts.tag_filter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::parse_filter;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(|l| l.to_string()).collect()
    }

    fn run(src: &str, args: &[&str]) -> Vec<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let filter = parse_filter(&args).unwrap();
        process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        )
        .lines
    }

    #[test]
    fn detect_open_double_slash() {
        let m = detect_marker("//version 1.2 *", CommentStyle::DoubleSlash);
        assert_eq!(
            m,
            MarkerKind::Versioned(Marker {
                version: "1.2".into(),
                to: None,
                tags: vec![]
            })
        );
    }

    #[test]
    fn detect_open_hash_no_star() {
        let m = detect_marker("#version 1.2", CommentStyle::Hash);
        assert_eq!(
            m,
            MarkerKind::Versioned(Marker {
                version: "1.2".into(),
                to: None,
                tags: vec![]
            })
        );
    }

    #[test]
    fn detect_open_with_tags() {
        let m = detect_marker(
            "//version 1.2 [inventory,combat] *",
            CommentStyle::DoubleSlash,
        );
        assert_eq!(
            m,
            MarkerKind::Versioned(Marker {
                version: "1.2".into(),
                to: None,
                tags: vec!["inventory".into(), "combat".into()]
            })
        );
    }

    #[test]
    fn detect_all_marker() {
        let m = detect_marker("//version ALL", CommentStyle::DoubleSlash);
        assert!(matches!(m, MarkerKind::All(_)));
    }

    #[test]
    fn detect_inline_range() {
        let m = detect_marker("//version 1.3 2.0", CommentStyle::DoubleSlash);
        assert_eq!(
            m,
            MarkerKind::InlineRange(Marker {
                version: "1.3".into(),
                to: Some("2.0".into()),
                tags: vec![],
            })
        );
    }

    #[test]
    fn detect_range_block_with_star() {
        let m = detect_marker("//version 1.3 2.0 *", CommentStyle::DoubleSlash);
        assert_eq!(
            m,
            MarkerKind::Versioned(Marker {
                version: "1.3".into(),
                to: Some("2.0".into()),
                tags: vec![],
            })
        );
    }

    #[test]
    fn detect_range_with_tags() {
        let m = detect_marker(
            "//version 1.3 2.0 [inventory,beta] *",
            CommentStyle::DoubleSlash,
        );
        assert_eq!(
            m,
            MarkerKind::Versioned(Marker {
                version: "1.3".into(),
                to: Some("2.0".into()),
                tags: vec!["inventory".into(), "beta".into()],
            })
        );
    }

    #[test]
    fn detect_range_from_ge_to_is_malformed() {
        let m = detect_marker("//version 2.0 1.3 *", CommentStyle::DoubleSlash);
        assert!(matches!(m, MarkerKind::Malformed(_)));
    }

    #[test]
    fn regular_comment_with_version_word_is_not_a_marker() {
        let m = detect_marker("// version 2 of foo", CommentStyle::DoubleSlash);
        assert!(matches!(m, MarkerKind::Malformed(_)));
    }

    #[test]
    fn versionish_is_not_a_marker() {
        let m = detect_marker("//versionish 1.2", CommentStyle::DoubleSlash);
        assert_eq!(m, MarkerKind::None);
    }

    #[test]
    fn flat_block_filtered_out() {
        let src = "before\n//version 1.2 *\ninside\n//version 1.2 *\nafter";
        assert_eq!(run(src, &["1.1"]), vec!["before", "after"]);
    }

    #[test]
    fn flat_block_included() {
        let src = "before\n//version 1.2 *\ninside\n//version 1.2 *\nafter";
        assert_eq!(run(src, &["1.2"]), vec!["before", "inside", "after"]);
    }

    #[test]
    fn nested_outer_fails_inner_discarded() {
        let src = "\
a
//version 1.5 *
b
//version 1.0 *
c
//version 1.0 *
d
//version 1.5 *
e";
        assert_eq!(run(src, &["1.2"]), vec!["a", "e"]);
    }

    #[test]
    fn inline_range_includes_next_line_only() {
        let src = "\
a
//version 1.3 2.0
inline_target
b";
        // Build 1.5 → 1.3 <= 1.5 < 2.0 passes → include `inline_target`.
        assert_eq!(run(src, &["1.5"]), vec!["a", "inline_target", "b"]);
        // Build 2.0 → boundary fails → skip `inline_target`.
        assert_eq!(run(src, &["2.0"]), vec!["a", "b"]);
        // Build 0.9 → below lower → skip.
        assert_eq!(run(src, &["0.9"]), vec!["a", "b"]);
    }

    #[test]
    fn inline_range_skipped_in_only_mode() {
        let src = "a\n//version 1.3 2.0\ninline_target\nb";
        // ONLY mode skips range markers; inline_target gets dropped.
        assert_eq!(run(src, &["1.5", "ONLY"]), vec!["a", "b"]);
    }

    #[test]
    fn range_block_open_close_paired_by_to() {
        let src = "\
a
//version 1.3 2.0 *
in
//version 1.3 2.0 *
b";
        // Build 1.5 → range covers → include `in`.
        assert_eq!(run(src, &["1.5"]), vec!["a", "in", "b"]);
        // Build 2.0 → upper exclusive → drop `in`.
        assert_eq!(run(src, &["2.0"]), vec!["a", "b"]);
    }

    #[test]
    fn range_block_nested_inside_regular_block() {
        let src = "\
//version 1.0 *
outer
//version 1.3 2.0 *
inner
//version 1.3 2.0 *
//version 1.0 *
tail";
        // Build 1.5 → outer passes (1.0<=1.5), range passes → both kept.
        assert_eq!(run(src, &["1.5"]), vec!["outer", "inner", "tail"]);
        // Build 2.5 → outer passes, range fails (2.5>=2.0) → only outer kept.
        assert_eq!(run(src, &["2.5"]), vec!["outer", "tail"]);
    }

    #[test]
    fn tag_filter_excludes_block_with_no_matching_tag() {
        let src = "\
base
//version 1.2 [combat] *
combat_code
//version 1.2 *
//version 1.2 *
untagged
//version 1.2 *
//version 1.2 [inventory] *
inv_code
//version 1.2 *
tail";
        let filter = parse_filter(&[String::from("2.0")]).unwrap();
        let opts = ProcessOptions {
            tag_filter: &[String::from("inventory")],
            extract_preserve_context: false,
            ..Default::default()
        };
        let r = process_file(&lines(src), CommentStyle::DoubleSlash, &filter, opts);
        assert_eq!(r.lines, vec!["base", "untagged", "inv_code", "tail"]);
    }

    #[test]
    fn extract_mode_drops_base() {
        let src = "\
base_top
//version 1.2 *
inside
//version 1.2 *
base_bottom";
        let filter = FilterMode::Extract(parse_version("1.2").unwrap());
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        assert_eq!(r.lines, vec!["inside"]);
    }

    #[test]
    fn extract_with_preserve_context_keeps_base() {
        let src = "\
base_top
//version 1.2 *
inside
//version 1.2 *
base_bottom";
        let filter = FilterMode::Extract(parse_version("1.2").unwrap());
        let opts = ProcessOptions {
            tag_filter: &[],
            extract_preserve_context: true,
            ..Default::default()
        };
        let r = process_file(&lines(src), CommentStyle::DoubleSlash, &filter, opts);
        assert_eq!(r.lines, vec!["base_top", "inside", "base_bottom"]);
    }

    #[test]
    fn malformed_collected_with_line_numbers() {
        let src = "ok\n//version notaversion *\nstill_ok";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        assert_eq!(r.malformed.len(), 1);
        assert_eq!(r.malformed[0].0, 2);
        assert_eq!(r.lines, vec!["ok", "still_ok"]);
    }

    #[test]
    fn malformed_unterminated_tags() {
        let m = detect_marker("//version 1.2 [oops", CommentStyle::DoubleSlash);
        assert!(matches!(m, MarkerKind::Malformed(_)));
    }

    #[test]
    fn unclosed_block_detected() {
        let src = "//version 1.2 *\ninside";
        let filter = parse_filter(&[String::from("1.2")]).unwrap();
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        assert_eq!(r.unclosed, vec![String::from("1.2")]);
    }

    #[test]
    fn unclosed_range_block_detected() {
        let src = "//version 1.3 2.0 *\ninside";
        let filter = parse_filter(&[String::from("1.5")]).unwrap();
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        assert_eq!(r.unclosed, vec![String::from("1.3 2.0")]);
    }

    #[test]
    fn exc_block_excluded_in_every_mode() {
        let src = "keep\n//version EXC\nsecret\n//version EXC\ntail";
        // Excluded under cumulative, only, and a wide range alike.
        assert_eq!(run(src, &["9.9"]), vec!["keep", "tail"]);
        assert_eq!(run(src, &["1.0", "ONLY"]), vec!["keep", "tail"]);
        assert_eq!(run(src, &["0.0", "9.9"]), vec!["keep", "tail"]);
    }

    #[test]
    fn exc_wins_over_passing_inner_block() {
        // Inner 1.0 would pass -v 1.0, but the EXC ancestor forces exclusion.
        let src = "\
a
//version EXC
//version 1.0 *
inner
//version 1.0 *
//version EXC
b";
        assert_eq!(run(src, &["1.0"]), vec!["a", "b"]);
    }

    #[test]
    fn strip_comments_removes_whole_line_comments() {
        let src = "// header comment\ncode1\n  // indented comment\ncode2\nlet u = \"http://x\"; // trailing kept";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let opts = ProcessOptions {
            strip_comments: true,
            ..Default::default()
        };
        let r = process_file(&lines(src), CommentStyle::DoubleSlash, &filter, opts);
        // Whole-line comments gone; code (incl. line with trailing comment) untouched.
        assert_eq!(
            r.lines,
            vec!["code1", "code2", "let u = \"http://x\"; // trailing kept"]
        );
    }

    #[test]
    fn all_block_contents_always_included() {
        let src = "x\n//version ALL\nkept\n//version ALL\ny";
        let filter = parse_filter(&[String::from("1.0"), String::from("ONLY")]).unwrap();
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        assert_eq!(r.lines, vec!["x", "kept", "y"]);
    }
}
