//! Per-version file and folder variants.
//!
//! A directory named `.vertion.<target>` holds several variants of one output
//! file (or folder). The variant that best matches the active build is copied
//! out as `<target>`; the directory itself never reaches the output.
//!
//! ```text
//! assets/.vertion.logo.png/
//!     0.0.0.png             ← fallback, matches any version
//!     2.0.0.png             ← from 2.0.0 onward
//!     1.2.3e2.0.0.png       ← 1.2.3 <= build < 2.0.0
//!     beta-combat.png       ← tag `beta` or `combat`, any version
//!     2.0.0-beta@ready.png  ← >= 2.0.0, tag `beta`, condition `ready`
//!     .vertion.default.png  ← used when nothing else matches
//! ```

use semver::Version;

use crate::filter::{tag_passes, FilterMode};
use crate::parser::{conditions_pass, parse_condition_token, MarkerCondition};

/// Directories starting with this hold variants; the rest of the name is the
/// output name, extension included (`.vertion.logo.png` → `logo.png`).
pub const VARIANT_PREFIX: &str = ".vertion.";

/// Reserved variant stem used when no other variant matches.
pub const DEFAULT_STEM: &str = ".vertion.default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantSpec {
    /// Lower bound, inclusive. `None` means "any version".
    pub min: Option<Version>,
    /// Upper bound, exclusive (the `e` separator).
    pub max: Option<Version>,
    pub tags: Vec<String>,
    pub conditions: Vec<MarkerCondition>,
    /// True for the reserved `.vertion.default` variant.
    pub is_default: bool,
}

/// Best (lowest) position of any of `tags` in the priority list; `usize::MAX`
/// when none of them are listed.
fn priority_index(tags: &[String], priority: &[String]) -> usize {
    tags.iter()
        .filter_map(|t| priority.iter().position(|p| p.eq_ignore_ascii_case(t)))
        .min()
        .unwrap_or(usize::MAX)
}

/// Comparable rank for "best variant wins". Compared lexicographically.
pub type VariantRank = (Version, std::cmp::Reverse<usize>, usize, usize);

impl VariantSpec {
    /// Rank for "best wins", compared lexicographically:
    ///
    /// 1. **Version** — unversioned variants sort lowest, so a versioned variant
    ///    always beats a bare-tag one.
    /// 2. **Tag priority** (`[project].tag_priority`) — an explicit statement of
    ///    which tag matters more, so it outranks the specificity heuristic below.
    ///    Unlisted tags rank last. `Reverse` because a lower index is better.
    /// 3. **Specificity** — more tags, then more conditions. At the same version
    ///    a variant carrying more of them is a deliberate override of the plainer
    ///    one (`2.0.0-beta.png` exists to replace `2.0.0.png` for beta builds).
    pub fn rank(&self, priority: &[String]) -> VariantRank {
        (
            self.min.clone().unwrap_or_else(|| Version::new(0, 0, 0)),
            std::cmp::Reverse(priority_index(&self.tags, priority)),
            self.tags.len(),
            self.conditions.len(),
        )
    }
}

/// `1.2.3` or `1.2.3e2.0.0`. Returns `None` when the segment isn't a version at
/// all (so it can be re-read as a tag — note `beta` contains an `e`, which is
/// exactly why this has to fail softly rather than error).
fn parse_version_segment(s: &str) -> Option<(Version, Option<Version>)> {
    if let Some(idx) = s.find('e') {
        if let (Ok(min), Ok(max)) = (
            crate::filter::parse_version(&s[..idx]),
            crate::filter::parse_version(&s[idx + 1..]),
        ) {
            return Some((min, Some(max)));
        }
    }
    crate::filter::parse_version(s).ok().map(|v| (v, None))
}

/// Parse a variant file/folder stem.
///
/// Grammar: `[<min>[e<max>]]` followed by `-<tag>[@<cond>...]` groups. The
/// leading `-` is only needed when something precedes, so a stem may start with
/// a tag directly (`beta.png`).
pub fn parse_variant_stem(stem: &str) -> Result<VariantSpec, String> {
    if stem == DEFAULT_STEM {
        return Ok(VariantSpec {
            min: None,
            max: None,
            tags: Vec::new(),
            conditions: Vec::new(),
            is_default: true,
        });
    }
    if stem.is_empty() {
        return Err("empty variant name".into());
    }

    let mut segments = stem.split('-');
    let first = segments.next().unwrap_or("");
    let mut spec = VariantSpec {
        min: None,
        max: None,
        tags: Vec::new(),
        conditions: Vec::new(),
        is_default: false,
    };

    // The first segment is the version when it parses as one, else a tag group.
    let mut pending: Vec<&str> = Vec::new();
    match parse_version_segment(first) {
        Some((min, max)) => {
            if let Some(m) = &max {
                if &min >= m {
                    return Err(format!("version range `{}` has min >= max", first));
                }
            }
            spec.min = Some(min);
            spec.max = max;
        }
        None => pending.push(first),
    }
    pending.extend(segments);

    for group in pending {
        if group.is_empty() {
            return Err(format!("empty tag in `{}`", stem));
        }
        let mut parts = group.split('@');
        let tag = parts.next().unwrap_or("").trim();
        if tag.is_empty() {
            return Err(format!("missing tag name before `@` in `{}`", stem));
        }
        spec.tags.push(tag.to_string());
        for cond in parts {
            spec.conditions
                .push(parse_condition_token(cond).map_err(|e| format!("{} in `{}`", e, stem))?);
        }
    }
    Ok(spec)
}

/// Whether a variant qualifies for this build.
///
/// The version window uses the same rule as range markers — `min <= target <
/// max` — evaluated against the filter's upper bound so every filter mode
/// (cumulative, range, only, include) resolves to a single comparison point.
pub fn spec_matches(
    spec: &VariantSpec,
    filter: &FilterMode,
    tag_filter: &[String],
    conditions: &[(String, bool)],
) -> bool {
    if spec.is_default {
        return false; // only used as an explicit fallback
    }
    let target = filter.upper();
    if let Some(min) = &spec.min {
        if target < min {
            return false;
        }
    }
    if let Some(max) = &spec.max {
        if target >= max {
            return false;
        }
    }
    tag_passes(&spec.tags, tag_filter) && conditions_pass(&spec.conditions, conditions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::parse_filter;

    fn v(s: &str) -> Version {
        crate::filter::parse_version(s).unwrap()
    }

    #[test]
    fn parses_plain_version() {
        let s = parse_variant_stem("2.0.0").unwrap();
        assert_eq!(s.min, Some(v("2.0.0")));
        assert_eq!(s.max, None);
        assert!(s.tags.is_empty());
    }

    #[test]
    fn parses_version_range() {
        let s = parse_variant_stem("1.2.3e2.0.0").unwrap();
        assert_eq!(s.min, Some(v("1.2.3")));
        assert_eq!(s.max, Some(v("2.0.0")));
    }

    #[test]
    fn parses_bare_tag_even_when_it_contains_an_e() {
        // `beta` must not be mistaken for a `b`e`ta` version range.
        let s = parse_variant_stem("beta").unwrap();
        assert_eq!(s.min, None);
        assert_eq!(s.tags, vec!["beta".to_string()]);
    }

    #[test]
    fn parses_version_with_tags_and_conditions() {
        let s = parse_variant_stem("2.0.0-beta@a@b").unwrap();
        assert_eq!(s.min, Some(v("2.0.0")));
        assert_eq!(s.tags, vec!["beta".to_string()]);
        assert_eq!(s.conditions.len(), 2);
        assert_eq!(s.conditions[0].name, "a");
        assert!(!s.conditions[0].negated);
    }

    #[test]
    fn parses_multiple_tags() {
        let s = parse_variant_stem("2.0.0-beta-combat").unwrap();
        assert_eq!(s.tags, vec!["beta".to_string(), "combat".to_string()]);
    }

    #[test]
    fn parses_negated_condition() {
        let s = parse_variant_stem("beta@!legacy").unwrap();
        assert!(s.conditions[0].negated);
        assert_eq!(s.conditions[0].name, "legacy");
    }

    #[test]
    fn default_stem_is_flagged_and_never_auto_matches() {
        let s = parse_variant_stem(DEFAULT_STEM).unwrap();
        assert!(s.is_default);
        let f = parse_filter(&[String::from("9.9")]).unwrap();
        assert!(!spec_matches(&s, &f, &[], &[]));
    }

    #[test]
    fn rejects_inverted_range_and_empty_parts() {
        assert!(parse_variant_stem("2.0.0e1.0.0").is_err());
        assert!(parse_variant_stem("2.0.0-").is_err());
        assert!(parse_variant_stem("2.0.0-@cond").is_err());
    }

    #[test]
    fn version_window_matches_like_a_range_marker() {
        let s = parse_variant_stem("1.2.3e2.0.0").unwrap();
        let inside = parse_filter(&[String::from("1.5")]).unwrap();
        let below = parse_filter(&[String::from("1.0")]).unwrap();
        let at_max = parse_filter(&[String::from("2.0.0")]).unwrap();
        assert!(spec_matches(&s, &inside, &[], &[]));
        assert!(!spec_matches(&s, &below, &[], &[]));
        assert!(!spec_matches(&s, &at_max, &[], &[])); // upper bound exclusive
    }

    #[test]
    fn tagged_variant_needs_an_active_tag() {
        let s = parse_variant_stem("2.0.0-beta").unwrap();
        let f = parse_filter(&[String::from("2.5")]).unwrap();
        assert!(!spec_matches(&s, &f, &[], &[]));
        assert!(spec_matches(&s, &f, &[String::from("beta")], &[]));
        // Wildcard admits every tagged variant.
        assert!(spec_matches(&s, &f, &[String::from("*")], &[]));
    }

    #[test]
    fn conditions_gate_the_variant() {
        let s = parse_variant_stem("2.0.0-beta@ready").unwrap();
        let f = parse_filter(&[String::from("2.5")]).unwrap();
        let tags = [String::from("beta")];
        assert!(!spec_matches(&s, &f, &tags, &[("ready".into(), false)]));
        assert!(spec_matches(&s, &f, &tags, &[("ready".into(), true)]));
    }

    #[test]
    fn versioned_outranks_unversioned() {
        let bare = parse_variant_stem("beta").unwrap();
        let versioned = parse_variant_stem("2.0.0").unwrap();
        assert!(versioned.rank(&[]) > bare.rank(&[]));
    }

    #[test]
    fn tag_priority_breaks_ties_between_equal_variants() {
        // Without a priority list these are indistinguishable — same version,
        // same tag count — which is the ambiguity the setting exists to resolve.
        let beta = parse_variant_stem("2.0.0-beta").unwrap();
        let combat = parse_variant_stem("2.0.0-combat").unwrap();
        assert_eq!(beta.rank(&[]), combat.rank(&[]));

        let priority = vec!["beta".to_string(), "combat".to_string()];
        assert!(beta.rank(&priority) > combat.rank(&priority));
        // Reversing the list reverses the winner.
        let flipped = vec!["combat".to_string(), "beta".to_string()];
        assert!(combat.rank(&flipped) > beta.rank(&flipped));
    }

    #[test]
    fn listed_tags_outrank_unlisted_ones() {
        let listed = parse_variant_stem("2.0.0-beta").unwrap();
        // Two tags but neither is listed: priority beats the specificity count.
        let unlisted = parse_variant_stem("2.0.0-x-y").unwrap();
        let priority = vec!["beta".to_string()];
        assert!(listed.rank(&priority) > unlisted.rank(&priority));
        // With no priority configured, specificity decides as before.
        assert!(unlisted.rank(&[]) > listed.rank(&[]));
    }

    #[test]
    fn a_variant_ranks_by_its_best_tag() {
        // `combat` is last, but the variant also carries `beta`, which is first.
        let mixed = parse_variant_stem("2.0.0-combat-beta").unwrap();
        let plain_combat = parse_variant_stem("2.0.0-combat").unwrap();
        let priority = vec!["beta".to_string(), "combat".to_string()];
        assert!(mixed.rank(&priority) > plain_combat.rank(&priority));
    }

    #[test]
    fn version_still_dominates_tag_priority() {
        let low_but_preferred = parse_variant_stem("1.0.0-beta").unwrap();
        let high_unlisted = parse_variant_stem("2.0.0-combat").unwrap();
        let priority = vec!["beta".to_string()];
        assert!(high_unlisted.rank(&priority) > low_but_preferred.rank(&priority));
    }

    #[test]
    fn tag_priority_is_case_insensitive() {
        let s = parse_variant_stem("2.0.0-Beta").unwrap();
        let other = parse_variant_stem("2.0.0-combat").unwrap();
        let priority = vec!["beta".to_string()];
        assert!(s.rank(&priority) > other.rank(&priority));
    }

    #[test]
    fn more_specific_wins_at_the_same_version() {
        // A tagged variant is a deliberate override of the plain one.
        let plain = parse_variant_stem("2.0.0").unwrap();
        let tagged = parse_variant_stem("2.0.0-beta").unwrap();
        let conditioned = parse_variant_stem("2.0.0-beta@ready").unwrap();
        assert!(tagged.rank(&[]) > plain.rank(&[]));
        assert!(conditioned.rank(&[]) > tagged.rank(&[]));
        // Version still dominates specificity.
        let higher = parse_variant_stem("3.0.0").unwrap();
        assert!(higher.rank(&[]) > conditioned.rank(&[]));
    }
}
