use crate::config::CommentStyle;
use crate::filter::{parse_version, passes, FilterMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// Raw token from the marker: a version string like "1.2" or the literal "ALL".
    pub version: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerKind {
    /// A versioned open or close marker; open vs close is decided by the stack at processing time.
    Versioned(Marker),
    /// Explicit `ALL` marker — a block whose contents are always included.
    All(Marker),
    /// Malformed marker: looks like a marker but couldn't be parsed.
    Malformed(String),
    /// Not a marker.
    None,
}

const KEYWORD: &str = "version";

/// Parse a single line and return its marker classification.
///
/// Marker grammar (after the comment prefix and optional whitespace):
///   `version` <ws> <token> [<ws> `[tag1,tag2,...]`] [<ws> `*`] <ws>*
/// where `<token>` is either `ALL` (case-insensitive) or a semver-ish version.
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

    // Pull the version-or-ALL token (first whitespace-delimited word, stopping at `[`).
    let token_end = cursor
        .find(|c: char| c.is_whitespace() || c == '[')
        .unwrap_or(cursor.len());
    let token = &cursor[..token_end];
    cursor = cursor[token_end..].trim_start();

    let is_all = token.eq_ignore_ascii_case("ALL");
    if !is_all && parse_version(token).is_err() {
        return MarkerKind::Malformed(format!("unparseable version `{}`", token));
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
    if let Some(rest_after_star) = cursor.strip_prefix('*') {
        cursor = rest_after_star.trim_start();
    }
    if !cursor.is_empty() {
        return MarkerKind::Malformed(format!("unexpected trailing content `{}`", cursor.trim()));
    }

    let marker = Marker {
        version: token.to_string(),
        tags,
    };
    if is_all {
        MarkerKind::All(marker)
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

    for (idx, line) in lines.iter().enumerate() {
        match detect_marker(line, style) {
            MarkerKind::Versioned(m) => {
                had_markers = true;
                stripped += 1;
                // Top-of-stack version match closes the block.
                if stack
                    .last()
                    .map(|t| t.version == m.version)
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
            MarkerKind::Malformed(reason) => {
                had_markers = true;
                stripped += 1;
                malformed.push((idx + 1, reason));
            }
            MarkerKind::None => {
                let include = decide_inclusion(&stack, filter, opts, extract);
                if include {
                    out.push(line.clone());
                } else {
                    stripped += 1;
                }
            }
        }
    }

    let unclosed = stack.into_iter().map(|m| m.version).collect();
    ProcessResult {
        lines: out,
        had_markers,
        unclosed,
        stripped,
        malformed,
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
        // Extract mode: any ancestor matching the target version (+ tag filter) wins.
        // ALL ancestors are passthrough only when preserve_context is set.
        let any_match = stack.iter().any(|m| {
            !m.version.eq_ignore_ascii_case("ALL")
                && passes(&m.version, &m.tags, filter, opts.tag_filter)
        });
        if any_match {
            return true;
        }
        let only_all = stack.iter().all(|m| m.version.eq_ignore_ascii_case("ALL"));
        return only_all && opts.extract_preserve_context;
    }
    // Cumulative / Range / Only: every ancestor must independently pass.
    stack
        .iter()
        .all(|m| passes(&m.version, &m.tags, filter, opts.tag_filter))
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
    fn regular_comment_with_version_word_is_not_a_marker() {
        let m = detect_marker("// version 2 of foo", CommentStyle::DoubleSlash);
        // After "version" comes " 2 of foo" — "2" is a valid version, but trailing
        // "of foo" remains, so the line is rejected as malformed (not silently a marker).
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
        };
        let r = process_file(&lines(src), CommentStyle::DoubleSlash, &filter, opts);
        // Untagged blocks pass tag filter; only the [combat] block is excluded.
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
