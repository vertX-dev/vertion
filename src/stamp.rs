//! Writing the build's version into JSON output files.
//!
//! A `[[stamp]]` entry names an output file and the keys inside it that hold a
//! version. After a build, each matching value is rewritten to the build's
//! version: a string becomes `"1.2.0"`, and a `[major, minor, patch]` array
//! (the Minecraft `manifest.json` style) becomes `[1, 2, 0]`.
//!
//! The file is edited as text, not re-serialized: only the matched values
//! change, so formatting, key order and comments survive, and the line count
//! never changes — `vertion map` relies on that to detect a stale build.

use semver::Version;

/// One segment of a key path such as `modules.*.version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    /// An object key — or an array index, when it is a number.
    Name(String),
    /// `*`: every member of an object, or every element of an array.
    Any,
}

/// A parsed `[[stamp]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// Output path, normalized like a `[[files]]` path.
    pub path: String,
    /// The keys as written in the config, for messages.
    pub keys: Vec<String>,
    /// The same keys, parsed. Parallel to `keys`.
    pub patterns: Vec<Vec<Seg>>,
}

/// Parse a dot-separated key path. `*` is a wildcard segment.
pub fn parse_key(key: &str) -> Result<Vec<Seg>, String> {
    if key.is_empty() {
        return Err("empty stamp key".into());
    }
    key.split('.')
        .map(|s| match s {
            "" => Err(format!("stamp key `{}` has an empty segment", key)),
            "*" => Ok(Seg::Any),
            s => Ok(Seg::Name(s.to_string())),
        })
        .collect()
}

/// The outcome of stamping one file's text.
#[derive(Debug)]
pub struct Report {
    pub text: String,
    /// Things a build should warn about: keys that matched nothing, or that
    /// matched a value which isn't a version.
    pub problems: Vec<String>,
}

/// Rewrite every value in `text` that `stamp`'s keys select to `version`.
///
/// Errors only when `text` isn't JSON at all; everything short of that is a
/// [`Report::problems`] entry, and the values that could be stamped still are.
pub fn stamp_text(text: &str, stamp: &Stamp, version: &Version) -> Result<Report, String> {
    let mut sc = Scanner {
        s: text.as_bytes(),
        i: 0,
        patterns: &stamp.patterns,
        path: Vec::new(),
        hits: vec![0; stamp.patterns.len()],
        edits: Vec::new(),
        problems: Vec::new(),
        version,
    };
    let parsed = sc.ws().and_then(|_| sc.value()).and_then(|_| sc.ws());
    if let Err(msg) = parsed {
        return Err(format!(
            "not valid JSON at line {}: {}",
            line_at(text, sc.i),
            msg
        ));
    }
    if sc.i != sc.s.len() {
        return Err(format!(
            "not valid JSON at line {}: unexpected content after the top-level value",
            line_at(text, sc.i)
        ));
    }

    let mut problems = sc.problems;
    for (key, hits) in stamp.keys.iter().zip(&sc.hits) {
        if *hits == 0 {
            problems.push(format!("stamp key `{}` matched nothing", key));
        }
    }

    // Two keys can select the same value; apply each span once, back to front
    // so earlier offsets stay valid.
    let mut edits = sc.edits;
    edits.sort_by_key(|e| e.0);
    edits.dedup_by_key(|e| e.0);
    let mut out = text.to_string();
    for (start, end, with) in edits.into_iter().rev() {
        out.replace_range(start..end, &with);
    }
    Ok(Report {
        text: out,
        problems,
    })
}

fn line_at(text: &str, pos: usize) -> usize {
    1 + text.as_bytes()[..pos.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

/// A step taken while walking the document.
enum Step {
    Key(String),
    Index(usize),
}

fn matches(pattern: &[Seg], path: &[Step]) -> bool {
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(path)
            .all(|(seg, step)| match (seg, step) {
                (Seg::Any, _) => true,
                (Seg::Name(n), Step::Key(k)) => n == k,
                (Seg::Name(n), Step::Index(i)) => n.parse::<usize>().ok() == Some(*i),
            })
}

/// What a parsed value was, and where. Arrays also report their direct
/// elements, which is all a `[major, minor, patch]` stamp needs.
struct Parsed {
    kind: &'static str,
    start: usize,
    end: usize,
    elems: Vec<(&'static str, usize, usize)>,
}

/// A recursive-descent JSON reader that records byte spans instead of building
/// a tree. Tolerates `//` and `/* */` comments and trailing commas, both of
/// which Minecraft's own JSON reader accepts.
struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
    patterns: &'a [Vec<Seg>],
    path: Vec<Step>,
    hits: Vec<usize>,
    edits: Vec<(usize, usize, String)>,
    problems: Vec<String>,
    version: &'a Version,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Skip whitespace and comments.
    fn ws(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.i += 1,
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'/') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'*') => {
                    let close = self.s[self.i + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .ok_or("unterminated /* comment")?;
                    self.i += 2 + close + 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == Some(b) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected `{}`", b as char))
        }
    }

    /// Parse one value, then stamp it if the current path is selected.
    fn value(&mut self) -> Result<Parsed, String> {
        let parsed = match self.peek() {
            Some(b'{') => self.object()?,
            Some(b'[') => self.array()?,
            Some(b'"') => {
                let start = self.i;
                self.string()?;
                self.leaf("string", start)
            }
            Some(b'-' | b'0'..=b'9') => {
                let start = self.i;
                while matches!(
                    self.peek(),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.i += 1;
                }
                self.leaf("number", start)
            }
            Some(b't' | b'f' | b'n') => {
                let start = self.i;
                let (word, kind) = [
                    ("true", "a boolean"),
                    ("false", "a boolean"),
                    ("null", "null"),
                ]
                .into_iter()
                .find(|(w, _)| self.s[self.i..].starts_with(w.as_bytes()))
                .ok_or("unexpected character")?;
                self.i += word.len();
                self.leaf(kind, start)
            }
            Some(_) => return Err("unexpected character".into()),
            None => return Err("unexpected end of file".into()),
        };

        let selected: Vec<usize> = (0..self.patterns.len())
            .filter(|&k| matches(&self.patterns[k], &self.path))
            .collect();
        for k in selected {
            self.hits[k] += 1;
            self.stamp(&parsed);
        }
        Ok(parsed)
    }

    fn leaf(&self, kind: &'static str, start: usize) -> Parsed {
        Parsed {
            kind,
            start,
            end: self.i,
            elems: Vec::new(),
        }
    }

    fn stamp(&mut self, v: &Parsed) {
        let numbers: Vec<(usize, usize)> = v
            .elems
            .iter()
            .filter(|e| e.0 == "number")
            .map(|e| (e.1, e.2))
            .collect();
        match v.kind {
            // Inside the quotes; a semver string never needs escaping.
            "string" => self
                .edits
                .push((v.start + 1, v.end - 1, self.version.to_string())),
            "array" if v.elems.len() == 3 && numbers.len() == 3 => {
                let parts = [self.version.major, self.version.minor, self.version.patch];
                for ((start, end), n) in numbers.into_iter().zip(parts) {
                    self.edits.push((start, end, n.to_string()));
                }
            }
            kind => {
                let what = match kind {
                    "array" => "an array that isn't [major, minor, patch]",
                    "object" => "an object",
                    "number" => "a bare number",
                    other => other,
                };
                self.problems.push(format!(
                    "`{}` at line {} is {}, not a version string or [major, minor, patch]; left as is",
                    self.path_string(),
                    line_at(std::str::from_utf8(self.s).unwrap_or(""), v.start),
                    what
                ));
            }
        }
    }

    fn path_string(&self) -> String {
        self.path
            .iter()
            .map(|s| match s {
                Step::Key(k) => k.clone(),
                Step::Index(i) => i.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".")
    }

    fn object(&mut self) -> Result<Parsed, String> {
        let start = self.i;
        self.i += 1;
        loop {
            self.ws()?;
            if self.peek() == Some(b'}') {
                self.i += 1;
                break;
            }
            if self.peek() != Some(b'"') {
                return Err("expected a key".into());
            }
            let key = self.string()?;
            self.ws()?;
            self.expect(b':')?;
            self.ws()?;
            self.path.push(Step::Key(key));
            self.value()?;
            self.path.pop();
            self.ws()?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err("expected `,` or `}`".into()),
            }
        }
        Ok(self.leaf("object", start))
    }

    fn array(&mut self) -> Result<Parsed, String> {
        let start = self.i;
        self.i += 1;
        let mut elems = Vec::new();
        loop {
            self.ws()?;
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            self.path.push(Step::Index(elems.len()));
            let v = self.value()?;
            self.path.pop();
            elems.push((v.kind, v.start, v.end));
            self.ws()?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err("expected `,` or `]`".into()),
            }
        }
        Ok(Parsed {
            elems,
            ..self.leaf("array", start)
        })
    }

    /// Consume a string literal, returning its decoded contents.
    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                Some(b'\\') => {
                    let esc = self
                        .s
                        .get(self.i + 1)
                        .copied()
                        .ok_or("unterminated string")?;
                    self.i += 2;
                    let c = match esc {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = self
                                .s
                                .get(self.i..self.i + 4)
                                .and_then(|h| std::str::from_utf8(h).ok())
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .ok_or("bad \\u escape")?;
                            self.i += 4;
                            char::from_u32(hex).unwrap_or('\u{FFFD}')
                        }
                        other => other as char, // `"`, `\`, `/`
                    };
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                Some(b) => {
                    out.push(b);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(keys: &[&str]) -> Stamp {
        Stamp {
            path: "manifest.json".into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            patterns: keys.iter().map(|k| parse_key(k).unwrap()).collect(),
        }
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    const MANIFEST: &str = r#"{
  // Minecraft tolerates comments
  "format_version": 2,
  "header": {
    "name": "Pack",
    "version": [1, 0, 0],
    "min_engine_version": [1, 20, 0]
  },
  "modules": [
    { "type": "data", "version": [1, 0, 0] },
    {
      "type": "script",
      "version": [
        1,
        0,
        0
      ],
    },
  ]
}
"#;

    #[test]
    fn stamps_arrays_in_place_without_moving_lines() {
        let r = stamp_text(
            MANIFEST,
            &stamp(&["header.version", "modules.*.version"]),
            &v("2.3.4"),
        )
        .unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.text.contains(r#""version": [2, 3, 4],"#));
        assert!(r
            .text
            .contains(r#"{ "type": "data", "version": [2, 3, 4] }"#));
        // A multi-line array keeps its layout, element by element.
        assert!(r.text.contains("\n        2,\n        3,\n        4\n"));
        // Unselected versions and everything else are untouched.
        assert!(r.text.contains(r#""min_engine_version": [1, 20, 0]"#));
        assert!(r.text.contains("// Minecraft tolerates comments"));
        assert_eq!(r.text.lines().count(), MANIFEST.lines().count());
    }

    #[test]
    fn stamps_strings_with_the_full_version() {
        let text = "{\"version\": \"0.0.1\", \"name\": \"x\"}";
        let r = stamp_text(text, &stamp(&["version"]), &v("1.2.0-beta.1")).unwrap();
        assert_eq!(r.text, "{\"version\": \"1.2.0-beta.1\", \"name\": \"x\"}");
    }

    #[test]
    fn numeric_segments_index_arrays() {
        let text = r#"{"modules": [{"version": "1"}, {"version": "1"}]}"#;
        let r = stamp_text(text, &stamp(&["modules.1.version"]), &v("3.0.0")).unwrap();
        assert_eq!(
            r.text,
            r#"{"modules": [{"version": "1"}, {"version": "3.0.0"}]}"#
        );
    }

    #[test]
    fn unmatched_keys_and_wrong_types_are_reported_not_fatal() {
        let text = r#"{"a": {"version": 5}, "b": {"version": "1.0.0"}}"#;
        let r = stamp_text(
            text,
            &stamp(&["a.version", "b.version", "c.version"]),
            &v("2.0.0"),
        )
        .unwrap();
        // `b` still gets stamped despite the problems elsewhere.
        assert!(r.text.contains(r#""b": {"version": "2.0.0"}"#));
        assert!(r.text.contains(r#""a": {"version": 5}"#));
        assert_eq!(r.problems.len(), 2, "{:?}", r.problems);
        assert!(r.problems[0].contains("`a.version`") && r.problems[0].contains("bare number"));
        assert!(r.problems[1].contains("`c.version` matched nothing"));
    }

    #[test]
    fn two_element_arrays_are_not_versions() {
        let r = stamp_text(r#"{"v": [1, 0]}"#, &stamp(&["v"]), &v("2.0.0")).unwrap();
        assert_eq!(r.text, r#"{"v": [1, 0]}"#);
        assert!(r.problems[0].contains("[major, minor, patch]"));
    }

    #[test]
    fn keys_with_escapes_are_decoded_before_matching() {
        let r = stamp_text(r#"{"version": "1"}"#, &stamp(&["version"]), &v("2.0.0")).unwrap();
        assert!(r.problems.is_empty());
        assert!(r.text.ends_with(r#""2.0.0"}"#));
    }

    #[test]
    fn invalid_json_is_an_error_with_a_line() {
        let err =
            stamp_text("{\n  \"a\": 1\n  \"b\": 2\n}", &stamp(&["a"]), &v("1.0.0")).unwrap_err();
        assert!(err.contains("line 3"), "{err}");
        assert!(stamp_text("{} {}", &stamp(&["a"]), &v("1.0.0")).is_err());
    }

    #[test]
    fn key_syntax() {
        assert_eq!(
            parse_key("modules.*.version").unwrap(),
            vec![
                Seg::Name("modules".into()),
                Seg::Any,
                Seg::Name("version".into())
            ]
        );
        assert!(parse_key("").is_err());
        assert!(parse_key("a..b").is_err());
        assert!(parse_key(".a").is_err());
    }
}
