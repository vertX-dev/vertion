use semver::Version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeEntry {
    pub from: Version,
    pub to: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterMode {
    Cumulative(Version),
    Range(Version, Version),
    Only(Version),
    Extract(Version),
    Include(Vec<IncludeEntry>),
}

impl FilterMode {
    /// Upper-bound version used for build folder naming.
    pub fn upper(&self) -> &Version {
        match self {
            FilterMode::Cumulative(v) | FilterMode::Only(v) | FilterMode::Extract(v) => v,
            FilterMode::Range(_, to) => to,
            FilterMode::Include(entries) => entries
                .iter()
                .map(|e| &e.to)
                .max()
                .expect("Include filter must have at least one entry"),
        }
    }

    /// Lower-bound version (only meaningful for Range / Include).
    #[allow(dead_code)]
    pub fn lower(&self) -> Option<&Version> {
        match self {
            FilterMode::Range(from, _) => Some(from),
            FilterMode::Include(entries) => entries.iter().map(|e| &e.from).min(),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            FilterMode::Cumulative(_) => "cumulative",
            FilterMode::Range(_, _) => "range",
            FilterMode::Only(_) => "only",
            FilterMode::Extract(_) => "extract",
            FilterMode::Include(_) => "include",
        }
    }

    /// True if a versioned block (by version only, ignoring tags) qualifies for this mode.
    pub fn version_matches(&self, v: &Version) -> bool {
        match self {
            FilterMode::Cumulative(max) => v <= max,
            FilterMode::Range(from, to) => v >= from && v <= to,
            FilterMode::Only(target) => v == target,
            FilterMode::Extract(target) => v == target,
            FilterMode::Include(entries) => entries.iter().any(|e| v >= &e.from && v <= &e.to),
        }
    }
}

/// Resolve `from` + optional `offset` (e.g. `1.2 + 2` minor steps) into a range.
/// Offset is interpreted at the same level as `from`'s lowest changing component:
/// for `1.2`, `+2` means `1.4`; for `1.2.3`, `+2` means `1.2.5`.
pub fn parse_include_range(
    version: &str,
    offset: Option<u64>,
) -> Result<IncludeEntry, FilterError> {
    let from_raw = version.trim();
    let from = parse_version(from_raw)?;
    let to = match offset {
        None => from.clone(),
        Some(0) => from.clone(),
        Some(n) => {
            // Decide bump dimension from the original token's dot count.
            let dots = from_raw.bytes().filter(|b| *b == b'.').count();
            match dots {
                0 => Version::new(from.major + n, 0, 0),
                1 => Version::new(from.major, from.minor + n, 0),
                _ => Version::new(from.major, from.minor, from.patch + n),
            }
        }
    };
    if from > to {
        return Err(FilterError(format!(
            "include range start {} is greater than end {}",
            from, to
        )));
    }
    Ok(IncludeEntry { from, to })
}

/// Add an entry to a list, skipping exact duplicates.
pub fn add_include_entry(entries: &mut Vec<IncludeEntry>, new: IncludeEntry) -> bool {
    if entries.iter().any(|e| e == &new) {
        return false;
    }
    entries.push(new);
    true
}

/// Remove or trim an entry. Rules:
/// * `from` and `to` match exactly → delete the entry
/// * Only `from` matches and `to` is within the entry's range → set entry.from = to
/// * Otherwise → error
pub fn remove_include_entry(
    entries: &mut Vec<IncludeEntry>,
    from: &Version,
    to: &Version,
) -> Result<(), FilterError> {
    // Exact match → delete.
    if let Some(idx) = entries.iter().position(|e| &e.from == from && &e.to == to) {
        entries.remove(idx);
        return Ok(());
    }
    // Trim lower portion: from matches, to is inside.
    if let Some(idx) = entries
        .iter()
        .position(|e| &e.from == from && to > from && to <= &e.to)
    {
        entries[idx].from = to.clone();
        return Ok(());
    }
    Err(FilterError(format!(
        "no include entry matched {}..{}",
        from, to
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncrementLevel {
    Major,
    Minor,
    Patch,
}

impl IncrementLevel {
    pub fn parse(s: &str) -> Option<IncrementLevel> {
        match s.to_ascii_lowercase().as_str() {
            "major" => Some(IncrementLevel::Major),
            "minor" => Some(IncrementLevel::Minor),
            "patch" => Some(IncrementLevel::Patch),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            IncrementLevel::Major => "major",
            IncrementLevel::Minor => "minor",
            IncrementLevel::Patch => "patch",
        }
    }
}

pub fn autoincrement(version: &Version, level: IncrementLevel) -> Version {
    match level {
        IncrementLevel::Major => Version::new(version.major + 1, 0, 0),
        IncrementLevel::Minor => Version::new(version.major, version.minor + 1, 0),
        IncrementLevel::Patch => Version::new(version.major, version.minor, version.patch + 1),
    }
}

#[derive(Debug)]
pub struct FilterError(pub String);

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FilterError {}

pub fn parse_version(s: &str) -> Result<Version, FilterError> {
    let s = s.trim();
    let dots = s.bytes().filter(|b| *b == b'.').count();
    let padded = match dots {
        0 => format!("{}.0.0", s),
        1 => format!("{}.0", s),
        _ => s.to_string(),
    };
    Version::parse(&padded).map_err(|e| FilterError(format!("invalid version `{}`: {}", s, e)))
}

pub fn parse_filter(args: &[String]) -> Result<FilterMode, FilterError> {
    match args.len() {
        1 => Ok(FilterMode::Cumulative(parse_version(&args[0])?)),
        2 => {
            if args[1].eq_ignore_ascii_case("ONLY") {
                Ok(FilterMode::Only(parse_version(&args[0])?))
            } else {
                let from = parse_version(&args[0])?;
                let to = parse_version(&args[1])?;
                if from > to {
                    return Err(FilterError(format!(
                        "range start {} is greater than end {}",
                        args[0], args[1]
                    )));
                }
                Ok(FilterMode::Range(from, to))
            }
        }
        n => Err(FilterError(format!(
            "expected 1 or 2 version arguments, got {}",
            n
        ))),
    }
}

/// Tag filter: OR-logic.
///
/// - Empty filter → every block passes.
/// - Untagged block → passes regardless of filter (tag filter only constrains
///   blocks that actually carry tags).
/// - Tagged block → must share at least one tag (case-insensitive) with the filter.
pub fn tag_passes(block_tags: &[String], tag_filter: &[String]) -> bool {
    if tag_filter.is_empty() {
        return true;
    }
    if block_tags.is_empty() {
        return true;
    }
    block_tags
        .iter()
        .any(|t| tag_filter.iter().any(|f| f.eq_ignore_ascii_case(t)))
}

/// Decide whether a single versioned block passes the filter.
/// `ALL` blocks (signalled by the token literal "ALL") always pass.
pub fn passes(
    version_token: &str,
    block_tags: &[String],
    mode: &FilterMode,
    tag_filter: &[String],
) -> bool {
    if version_token.eq_ignore_ascii_case("ALL") {
        return tag_passes(block_tags, tag_filter);
    }
    let v = match parse_version(version_token) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if !mode.version_matches(&v) {
        return false;
    }
    tag_passes(block_tags, tag_filter)
}

// Back-compat helper for code paths that don't care about tags.
#[allow(dead_code)]
pub fn passes_filter(version_token: &str, mode: &FilterMode) -> bool {
    passes(version_token, &[], mode, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn cumulative_includes_lower_and_equal() {
        let f = parse_filter(&[s("1.2")]).unwrap();
        assert!(passes_filter("1.0", &f));
        assert!(passes_filter("1.2", &f));
        assert!(!passes_filter("1.3", &f));
    }

    #[test]
    fn range_inclusive_both_ends() {
        let f = parse_filter(&[s("1.1"), s("1.3")]).unwrap();
        assert!(!passes_filter("1.0", &f));
        assert!(passes_filter("1.1", &f));
        assert!(passes_filter("1.3", &f));
        assert!(!passes_filter("1.4", &f));
    }

    #[test]
    fn only_exact_match() {
        let f = parse_filter(&[s("1.2"), s("ONLY")]).unwrap();
        assert!(passes_filter("1.2", &f));
        assert!(!passes_filter("1.1", &f));
    }

    #[test]
    fn all_always_passes_no_tag_filter() {
        let f = parse_filter(&[s("1.2"), s("ONLY")]).unwrap();
        assert!(passes("ALL", &[], &f, &[]));
    }

    #[test]
    fn all_subject_to_tag_filter() {
        // Untagged ALL blocks pass the tag filter (they're treated like base).
        let f = parse_filter(&[s("1.2")]).unwrap();
        assert!(passes("ALL", &[], &f, &[s("inventory")]));
        // Tagged ALL blocks must match the filter.
        let tagged = vec![s("inventory")];
        assert!(passes("ALL", &tagged, &f, &[s("inventory")]));
        assert!(!passes("ALL", &[s("combat")], &f, &[s("inventory")]));
    }

    #[test]
    fn tag_or_logic() {
        let f = parse_filter(&[s("2.0")]).unwrap();
        let block = vec![s("combat")];
        assert!(passes("1.2", &block, &f, &[s("inventory"), s("combat")]));
        assert!(!passes("1.2", &block, &f, &[s("inventory")]));
    }

    #[test]
    fn extract_only_matches_exact_version() {
        let f = FilterMode::Extract(parse_version("1.2").unwrap());
        assert!(passes("1.2", &[], &f, &[]));
        assert!(!passes("1.1", &[], &f, &[]));
        assert!(!passes("1.3", &[], &f, &[]));
    }

    #[test]
    fn include_single_entry_acts_like_range() {
        let entry = parse_include_range("1.2", Some(2)).unwrap();
        assert_eq!(entry.from, Version::new(1, 2, 0));
        assert_eq!(entry.to, Version::new(1, 4, 0));
        let f = FilterMode::Include(vec![entry]);
        assert!(passes_filter("1.2", &f));
        assert!(passes_filter("1.4", &f));
        assert!(!passes_filter("1.5", &f));
        assert!(!passes_filter("1.1", &f));
    }

    #[test]
    fn include_union_with_gap() {
        let a = parse_include_range("1.1", None).unwrap();
        let b = parse_include_range("1.5", Some(3)).unwrap();
        let f = FilterMode::Include(vec![a, b]);
        assert!(passes_filter("1.1", &f));
        assert!(!passes_filter("1.2", &f));
        assert!(passes_filter("1.5", &f));
        assert!(passes_filter("1.8", &f));
        assert!(!passes_filter("1.9", &f));
    }

    #[test]
    fn include_add_skips_exact_duplicate() {
        let mut list = vec![parse_include_range("1.2", None).unwrap()];
        let added = add_include_entry(&mut list, parse_include_range("1.2", None).unwrap());
        assert!(!added);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn include_remove_trim_and_delete() {
        let mut list = vec![parse_include_range("1.2", Some(4)).unwrap()];
        // Trim lower portion: 1.2..1.4 → entry becomes 1.4..1.6
        remove_include_entry(
            &mut list,
            &parse_version("1.2").unwrap(),
            &parse_version("1.4").unwrap(),
        )
        .unwrap();
        assert_eq!(list[0].from, Version::new(1, 4, 0));
        assert_eq!(list[0].to, Version::new(1, 6, 0));
        // Full delete: 1.4..1.6
        remove_include_entry(
            &mut list,
            &parse_version("1.4").unwrap(),
            &parse_version("1.6").unwrap(),
        )
        .unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn include_remove_no_match_errors() {
        let mut list = vec![parse_include_range("1.2", None).unwrap()];
        let r = remove_include_entry(
            &mut list,
            &parse_version("9.9").unwrap(),
            &parse_version("9.9").unwrap(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn autoincrement_levels() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(
            autoincrement(&v, IncrementLevel::Major),
            Version::new(2, 0, 0)
        );
        assert_eq!(
            autoincrement(&v, IncrementLevel::Minor),
            Version::new(1, 3, 0)
        );
        assert_eq!(
            autoincrement(&v, IncrementLevel::Patch),
            Version::new(1, 2, 4)
        );
    }
}
