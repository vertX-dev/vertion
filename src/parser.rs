use crate::config::CommentStyle;
use crate::filter::{parse_version, passes, passes_range_marker, tag_passes, FilterMode};

/// One condition reference on a marker tag or `[[files]]` entry, e.g. `{cond}`
/// or the negated `{!cond}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerCondition {
    pub name: String,
    pub negated: bool,
}

impl MarkerCondition {
    /// Render back to source form: `name`, or `!name` when negated.
    pub fn display(&self) -> String {
        if self.negated {
            format!("!{}", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// Parse one condition token (`name` or `!name`), without the surrounding braces.
pub fn parse_condition_token(body: &str) -> Result<MarkerCondition, String> {
    let body = body.trim();
    let (negated, name) = match body.strip_prefix('!') {
        Some(rest) => (true, rest.trim()),
        None => (false, body),
    };
    if name.is_empty() {
        return Err("empty condition name".into());
    }
    if name.contains('!') || name.contains('{') || name.contains('}') {
        return Err(format!("invalid condition name `{}`", name));
    }
    Ok(MarkerCondition {
        name: name.to_string(),
        negated,
    })
}

/// Look up a resolved condition value by name.
pub fn lookup(name: &str, resolved: &[(String, bool)]) -> Option<bool> {
    resolved.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
}

/// Every condition must hold (AND). Negated conditions invert the value.
///
/// An **unknown** condition never passes, negated or not — otherwise a typo in
/// `{!typo}` would silently *include* code, which is the dangerous direction.
pub fn conditions_pass(conds: &[MarkerCondition], resolved: &[(String, bool)]) -> bool {
    conds.iter().all(|c| match lookup(&c.name, resolved) {
        None => false,
        Some(v) => v != c.negated,
    })
}

/// Parse one `[tag]` entry: a tag name followed by zero or more
/// `{[!]condition}` groups, e.g. `stable`, `stable{a}`, `stable{a}{!b}`.
fn parse_tag_entry(entry: &str) -> Result<(String, Vec<MarkerCondition>), String> {
    let (name, mut rest) = match entry.find('{') {
        Some(i) => (entry[..i].trim(), &entry[i..]),
        None => (entry.trim(), ""),
    };
    if name.is_empty() {
        return Err("missing tag name before `{`".into());
    }
    if name.contains('}') {
        return Err(format!(
            "stray `}}` in tag `{}` (conditions are written `tag{{name}}`)",
            name
        ));
    }
    let mut conds = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with('{') {
            return Err(format!(
                "unexpected `{}` after conditions on tag `{}`",
                rest.trim(),
                name
            ));
        }
        let Some(close) = rest.find('}') else {
            return Err(format!("unterminated `{{` condition on tag `{}`", name));
        };
        let cond = parse_condition_token(&rest[1..close])
            .map_err(|e| format!("{} on tag `{}`", e, name))?;
        conds.push(cond);
        rest = &rest[close + 1..];
    }
    Ok((name.to_string(), conds))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// First token: a version string like "1.2", or the literal "ALL"/"EXC".
    /// Empty for tag-only markers (`//version [wiki]`), which carry no version.
    pub version: String,
    /// Upper bound for range markers (`//version 1.3 2.0`). `None` for single-version markers.
    pub to: Option<String>,
    pub tags: Vec<String>,
    /// Conditions attached to this marker's tags (`[stable{imagesInStable}]`).
    /// All of them must hold for the marker to pass.
    pub conditions: Vec<MarkerCondition>,
}

impl Marker {
    /// True when this marker has no version token — selection is by tag alone.
    pub fn is_tag_only(&self) -> bool {
        self.version.is_empty()
    }

    /// Display label: `1.2`, `1.3 2.0` for ranges, or `[tags]` for tag-only.
    pub fn label(&self) -> String {
        if self.is_tag_only() {
            return self.pair_key();
        }
        match &self.to {
            Some(to) => format!("{} {}", self.version, to),
            None => self.version.clone(),
        }
    }

    /// Key that stands in for the version when pairing open/close markers.
    /// Identical to `version` except for tag-only markers, which have none and
    /// pair on their tag list instead. Callers track `to` separately.
    pub fn pair_key(&self) -> String {
        if self.is_tag_only() {
            format!("[{}]", self.tags.join(","))
        } else {
            self.version.clone()
        }
    }
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
    /// Tag-only marker (`//version [wiki]`) — no version, selection by tag alone.
    TagOnly(Marker),
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
///   `version` <ws> [<v1> [<ws> <v2>]] [<ws> `[tag1,tag2{cond},...]`] [<ws> `*`] <ws>*
///
/// Where:
/// - `<v1>` is `ALL` / `EXC` (case-insensitive) or a semver-ish version. It may be
///   omitted entirely when a `[tag]` list follows — a tag-only marker.
/// - `<v2>` is an optional upper bound for range markers (only valid when `<v1>` is a version).
///   - With `*`: range block (open/close paired by stack).
///   - Without `*`: inline range (applies to the next line only).
/// - A tag may carry a condition in braces (`stable{imagesInStable}`); every
///   condition on the marker must resolve true for it to pass.
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
            return MarkerKind::Malformed(
                "missing version, `ALL`/`EXC`, or `[tags]` after `version`".into(),
            );
        }
        Some(c) if !c.is_whitespace() => return MarkerKind::None,
        _ => {}
    }

    let mut cursor = after_kw.trim_start();
    if cursor.is_empty() {
        return MarkerKind::Malformed(
            "missing version, `ALL`/`EXC`, or `[tags]` after `version`".into(),
        );
    }

    // Tag-only marker: the version token is omitted and a `[tag]` list follows.
    let tag_only = cursor.starts_with('[');

    // First token: ALL, EXC, or a version (absent for tag-only markers).
    let mut token = "";
    let mut is_all = false;
    let mut is_exc = false;
    if !tag_only {
        // `*` terminates the token too, so `//version 1.2*` (no space) parses.
        let token_end = cursor
            .find(|c: char| c.is_whitespace() || c == '[' || c == '*')
            .unwrap_or(cursor.len());
        token = &cursor[..token_end];
        cursor = cursor[token_end..].trim_start();

        is_all = token.eq_ignore_ascii_case("ALL");
        is_exc = token.eq_ignore_ascii_case("EXC");
        if !(is_all || is_exc) && parse_version(token).is_err() {
            return MarkerKind::Malformed(format!("unparseable version `{}`", token));
        }
    }
    let is_keyword = is_all || is_exc;

    // Optional second version token (range marker upper bound). Only valid for a version.
    let mut to_token: Option<String> = None;
    if !tag_only
        && !is_keyword
        && !cursor.is_empty()
        && !cursor.starts_with('[')
        && !cursor.starts_with('*')
    {
        let next_end = cursor
            .find(|c: char| c.is_whitespace() || c == '[' || c == '*')
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

    // Optional [tags], each optionally carrying one or more `{condition}` groups.
    let mut tags: Vec<String> = Vec::new();
    let mut conditions: Vec<MarkerCondition> = Vec::new();
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
            match parse_tag_entry(t) {
                Ok((name, mut conds)) => {
                    tags.push(name);
                    conditions.append(&mut conds);
                }
                Err(reason) => return MarkerKind::Malformed(reason),
            }
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
        conditions,
    };
    if tag_only {
        MarkerKind::TagOnly(marker)
    } else if is_all {
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
    /// Markers referencing a condition name that isn't defined anywhere.
    pub unknown_conditions: Vec<(usize, String)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessOptions<'a> {
    pub tag_filter: &'a [String],
    pub extract_preserve_context: bool,
    /// Strip whole-line comments (non-marker) from included output.
    pub strip_comments: bool,
    /// Resolved condition values, as `(name, value)` pairs. An unknown name
    /// reads as false (and is reported via `unknown_conditions`).
    pub conditions: &'a [(String, bool)],
}

/// Every condition on the marker must hold (see `conditions::conditions_pass`).
fn conditions_hold(m: &Marker, conditions: &[(String, bool)]) -> bool {
    conditions_pass(&m.conditions, conditions)
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
    let mut unknown_conditions: Vec<(usize, String)> = Vec::new();
    let extract = matches!(filter, FilterMode::Extract(_));
    // Inline range pending: forces the next *content* line's inclusion to this value.
    let mut inline_pending: Option<bool> = None;

    // Report each unresolvable condition once, on the marker line that names it.
    let note_unknown = |idx: usize, m: &Marker, sink: &mut Vec<(usize, String)>| {
        for c in &m.conditions {
            if lookup(&c.name, opts.conditions).is_none() {
                sink.push((idx + 1, c.name.clone()));
            }
        }
    };

    for (idx, line) in lines.iter().enumerate() {
        match detect_marker(line, style) {
            MarkerKind::Versioned(m) => {
                had_markers = true;
                stripped += 1;
                note_unknown(idx, &m, &mut unknown_conditions);
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
            MarkerKind::TagOnly(m) => {
                had_markers = true;
                stripped += 1;
                note_unknown(idx, &m, &mut unknown_conditions);
                // Tag-only markers have no version to pair on, so they pair on
                // an identical tag+condition list instead.
                if stack
                    .last()
                    .map(|t| t.is_tag_only() && t.tags == m.tags && t.conditions == m.conditions)
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
                note_unknown(idx, &m, &mut unknown_conditions);
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
                note_unknown(idx, &m, &mut unknown_conditions);
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
                note_unknown(idx, &m, &mut unknown_conditions);
                // Combined: all ancestors pass AND this range marker passes.
                let ancestors_pass = stack
                    .iter()
                    .all(|a| marker_passes_filter(a, filter, opts, extract));
                let self_pass = conditions_hold(&m, opts.conditions)
                    && passes_range_marker(
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

    let unclosed = stack.into_iter().map(|m| m.label()).collect();
    ProcessResult {
        lines: out,
        had_markers,
        unclosed,
        stripped,
        malformed,
        unknown_conditions,
    }
}

/// Whether a marker on the ancestor stack passes the active filter.
/// Handles plain versioned, ALL, tag-only, and range-block markers uniformly.
fn marker_passes_filter(
    m: &Marker,
    filter: &FilterMode,
    opts: ProcessOptions<'_>,
    extract: bool,
) -> bool {
    if extract {
        // In extract mode the ancestor rule is evaluated separately by decide_inclusion;
        // this helper is only used for inline range eval, where extract is handled
        // by the caller. Default to ancestor passing for non-extract code paths.
        return passes_marker_general(m, filter, opts);
    }
    passes_marker_general(m, filter, opts)
}

fn passes_marker_general(m: &Marker, filter: &FilterMode, opts: ProcessOptions<'_>) -> bool {
    // A failing condition vetoes the marker regardless of version/tags.
    if !conditions_hold(m, opts.conditions) {
        return false;
    }
    if m.is_tag_only() {
        // No version gate — selection is by tag alone.
        return tag_passes(&m.tags, opts.tag_filter);
    }
    if let Some(to) = &m.to {
        passes_range_marker(&m.version, to, &m.tags, filter, opts.tag_filter)
    } else {
        passes(&m.version, &m.tags, filter, opts.tag_filter)
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
    // A failing condition anywhere in the chain vetoes the line in every mode.
    if !stack.iter().all(|m| conditions_hold(m, opts.conditions)) {
        return false;
    }
    if extract {
        // Extract mode: any non-ALL, non-range ancestor matching the target version wins.
        let any_match = stack.iter().any(|m| {
            !m.is_tag_only()
                && !m.version.eq_ignore_ascii_case("ALL")
                && m.to.is_none()
                && passes(&m.version, &m.tags, filter, opts.tag_filter)
        });
        if any_match {
            return true;
        }
        // ALL-only (or range-/tag-only) ancestor chain → only preserve_context emits.
        let only_passthrough = stack
            .iter()
            .all(|m| m.version.eq_ignore_ascii_case("ALL") || m.to.is_some() || m.is_tag_only());
        return only_passthrough && opts.extract_preserve_context;
    }
    // Cumulative / Range / Only / Include: every ancestor must independently pass.
    stack.iter().all(|m| passes_marker_general(m, filter, opts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::parse_filter;

    /// Wildcard tag filter — admits every tagged block.
    fn any_tag() -> Vec<String> {
        vec![String::from("*")]
    }

    fn cond(name: &str) -> MarkerCondition {
        MarkerCondition {
            name: name.to_string(),
            negated: false,
        }
    }

    fn ncond(name: &str) -> MarkerCondition {
        MarkerCondition {
            name: name.to_string(),
            negated: true,
        }
    }

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
                tags: vec![],
                conditions: vec![],
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
                tags: vec![],
                conditions: vec![],
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
                tags: vec!["inventory".into(), "combat".into()],
                conditions: vec![],
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
                conditions: vec![],
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
                conditions: vec![],
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
                conditions: vec![],
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
    fn detect_tag_only_marker() {
        let m = detect_marker("//version [wiki]", CommentStyle::DoubleSlash);
        assert_eq!(
            m,
            MarkerKind::TagOnly(Marker {
                version: String::new(),
                to: None,
                tags: vec!["wiki".into()],
                conditions: vec![],
            })
        );
        // `*` is allowed but optional on tag-only markers.
        assert!(matches!(
            detect_marker("#version [wiki] *", CommentStyle::Hash),
            MarkerKind::TagOnly(_)
        ));
    }

    #[test]
    fn detect_tag_condition() {
        let m = detect_marker(
            "//version [stable{imagesInStable}]",
            CommentStyle::DoubleSlash,
        );
        assert_eq!(
            m,
            MarkerKind::TagOnly(Marker {
                version: String::new(),
                to: None,
                tags: vec!["stable".into()],
                conditions: vec![cond("imagesInStable")],
            })
        );
        // Conditions also work alongside a version.
        let v = detect_marker("//version 1.2 [a{c1},b] *", CommentStyle::DoubleSlash);
        match v {
            MarkerKind::Versioned(m) => {
                assert_eq!(m.tags, vec!["a".to_string(), "b".to_string()]);
                assert_eq!(m.conditions, vec![cond("c1")]);
            }
            other => panic!("expected Versioned, got {:?}", other),
        }
    }

    #[test]
    fn star_may_be_glued_to_the_version() {
        assert_eq!(
            detect_marker("//version 1.2*", CommentStyle::DoubleSlash),
            MarkerKind::Versioned(Marker {
                version: "1.2".into(),
                to: None,
                tags: vec![],
                conditions: vec![],
            })
        );
        assert_eq!(
            detect_marker("//version 1.3 2.0*", CommentStyle::DoubleSlash),
            MarkerKind::Versioned(Marker {
                version: "1.3".into(),
                to: Some("2.0".into()),
                tags: vec![],
                conditions: vec![],
            })
        );
    }

    #[test]
    fn detect_negated_and_multiple_conditions() {
        let m = detect_marker("//version [stable{a}{!b}]", CommentStyle::DoubleSlash);
        match m {
            MarkerKind::TagOnly(m) => {
                assert_eq!(m.tags, vec!["stable".to_string()]);
                assert_eq!(m.conditions, vec![cond("a"), ncond("b")]);
            }
            other => panic!("expected TagOnly, got {:?}", other),
        }
        // Whitespace between groups and after `!` is tolerated.
        let m2 = detect_marker("//version [stable{a} {! b}] *", CommentStyle::DoubleSlash);
        match m2 {
            MarkerKind::TagOnly(m) => assert_eq!(m.conditions, vec![cond("a"), ncond("b")]),
            other => panic!("expected TagOnly, got {:?}", other),
        }
    }

    #[test]
    fn negated_condition_inverts() {
        let src = "base\n//version [x{!off}]\nbody\n//version [x{!off}]\ntail";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        // off = false → !off is true → included.
        let off = [("off".to_string(), false)];
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions {
                conditions: &off,
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert_eq!(r.lines, vec!["base", "body", "tail"]);
        // off = true → !off is false → excluded.
        let on = [("off".to_string(), true)];
        let r2 = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions {
                conditions: &on,
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert_eq!(r2.lines, vec!["base", "tail"]);
    }

    #[test]
    fn multiple_conditions_are_anded() {
        let src = "//version [x{a}{b}]\nbody\n//version [x{a}{b}]";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let run_with = |pairs: &[(String, bool)]| {
            process_file(
                &lines(src),
                CommentStyle::DoubleSlash,
                &filter,
                ProcessOptions {
                    conditions: pairs,
                    tag_filter: &any_tag(),
                    ..Default::default()
                },
            )
            .lines
        };
        let both = [("a".to_string(), true), ("b".to_string(), true)];
        assert_eq!(run_with(&both), vec!["body"]);
        let one = [("a".to_string(), true), ("b".to_string(), false)];
        assert!(run_with(&one).is_empty());
    }

    #[test]
    fn unknown_condition_never_passes_even_negated() {
        // A typo'd `{!name}` must not silently include the block.
        let src = "//version [x{!nosuch}]\nbody\n//version [x{!nosuch}]";
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &parse_filter(&[String::from("1.0")]).unwrap(),
            ProcessOptions::default(),
        );
        assert!(r.lines.is_empty());
        assert_eq!(
            r.unknown_conditions,
            vec![(1, "nosuch".to_string()), (3, "nosuch".to_string())]
        );
    }

    #[test]
    fn malformed_condition_syntax() {
        for line in [
            "//version [stable{oops]",
            "//version [{noname}]",
            "//version [stable{}]",
            "//version [stable}]",
            "//version [stable{!}]",
            "//version [stable{a}junk]",
            "//version [stable{!!a}]",
        ] {
            assert!(
                matches!(
                    detect_marker(line, CommentStyle::DoubleSlash),
                    MarkerKind::Malformed(_)
                ),
                "expected malformed for `{}`",
                line
            );
        }
    }

    #[test]
    fn tag_only_block_is_opt_in() {
        let src = "base
//version [wiki]
wiki_body
//version [wiki]
tail";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let run_tags = |tags: &[String]| {
            process_file(
                &lines(src),
                CommentStyle::DoubleSlash,
                &filter,
                ProcessOptions {
                    tag_filter: tags,
                    ..Default::default()
                },
            )
            .lines
        };
        // No tags active -> tagged content is skipped.
        assert_eq!(run_tags(&[]), vec!["base", "tail"]);
        // Naming the tag activates it.
        assert_eq!(
            run_tags(&[String::from("wiki")]),
            vec!["base", "wiki_body", "tail"]
        );
        // Wildcard admits every tag.
        assert_eq!(
            run_tags(&[String::from("*")]),
            vec!["base", "wiki_body", "tail"]
        );
        // A different tag leaves it out.
        assert_eq!(run_tags(&[String::from("other")]), vec!["base", "tail"]);
    }

    #[test]
    fn tag_only_blocks_pair_by_tag_list() {
        // Two adjacent tag-only blocks with different tags must not cross-pair.
        let src = "\
//version [a]
in_a
//version [a]
//version [b]
in_b
//version [b]";
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &parse_filter(&[String::from("1.0")]).unwrap(),
            ProcessOptions {
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert!(r.unclosed.is_empty());
        assert_eq!(r.lines, vec!["in_a", "in_b"]);
    }

    #[test]
    fn condition_gates_block() {
        let src = "base\n//version [stable{imagesInStable}]\nimg\n//version [stable{imagesInStable}]\ntail";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();

        let on = [("imagesInStable".to_string(), true)];
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions {
                conditions: &on,
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert_eq!(r.lines, vec!["base", "img", "tail"]);

        let off = [("imagesInStable".to_string(), false)];
        let r2 = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions {
                conditions: &off,
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert_eq!(r2.lines, vec!["base", "tail"]);
    }

    #[test]
    fn condition_gates_versioned_block_and_unknown_reads_false() {
        let src = "base\n//version 1.0 [x{nope}] *\nbody\n//version 1.0 *\ntail";
        let filter = parse_filter(&[String::from("1.0")]).unwrap();
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &filter,
            ProcessOptions::default(),
        );
        // Unknown condition → false → block excluded, and reported once.
        assert_eq!(r.lines, vec!["base", "tail"]);
        assert_eq!(r.unknown_conditions, vec![(2, "nope".to_string())]);
    }

    #[test]
    fn failing_condition_on_ancestor_vetoes_children() {
        let src = "\
//version [outer{off}]
//version 1.0 *
inner
//version 1.0 *
//version [outer{off}]";
        let off = [("off".to_string(), false)];
        let r = process_file(
            &lines(src),
            CommentStyle::DoubleSlash,
            &parse_filter(&[String::from("1.0")]).unwrap(),
            ProcessOptions {
                conditions: &off,
                tag_filter: &any_tag(),
                ..Default::default()
            },
        );
        assert!(r.lines.is_empty());
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
