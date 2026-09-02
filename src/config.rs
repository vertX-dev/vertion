//! Which line-comment syntax a source file uses.
//!
//! Vertion's markers live inside ordinary line comments, so before a file can
//! be scanned it has to be known whether they are spelt `//version 1.2` or
//! `#version 1.2`. The decision is made from the file extension alone.

/// The line-comment style a file uses, which decides how its markers are spelt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `//` — C-like languages: JavaScript, TypeScript, Rust, Java, Go, and so on.
    DoubleSlash,
    /// `#` — shell-like languages: Python, Ruby, YAML, TOML, and so on.
    Hash,
}

impl CommentStyle {
    /// The literal prefix a comment starts with: `"//"` or `"#"`.
    pub fn prefix(&self) -> &'static str {
        match self {
            CommentStyle::DoubleSlash => "//",
            CommentStyle::Hash => "#",
        }
    }
}

/// Pick a comment style from a file extension, with or without its leading dot.
///
/// Matching is case-insensitive. An extension that is not recognised falls back
/// to [`CommentStyle::DoubleSlash`], which is the more common of the two; a file
/// with no markers is copied through untouched either way, so the fallback only
/// matters for extensions that do carry markers.
pub fn detect_comment_style(extension: &str) -> CommentStyle {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "js" | "jsx" | "ts" | "tsx" | "rs" | "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "java"
        | "cs" | "go" | "kt" | "swift" | "scala" | "php" => CommentStyle::DoubleSlash,
        "py" | "sh" | "bash" | "zsh" | "rb" | "yaml" | "yml" | "toml" | "pl" | "r" => {
            CommentStyle::Hash
        }
        _ => CommentStyle::DoubleSlash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_double_slash() {
        assert_eq!(detect_comment_style("rs"), CommentStyle::DoubleSlash);
        assert_eq!(detect_comment_style(".js"), CommentStyle::DoubleSlash);
        assert_eq!(detect_comment_style("CPP"), CommentStyle::DoubleSlash);
    }

    #[test]
    fn known_hash() {
        assert_eq!(detect_comment_style("py"), CommentStyle::Hash);
        assert_eq!(detect_comment_style(".yml"), CommentStyle::Hash);
    }

    #[test]
    fn unknown_defaults_to_double_slash() {
        assert_eq!(detect_comment_style("xyz"), CommentStyle::DoubleSlash);
        assert_eq!(detect_comment_style(""), CommentStyle::DoubleSlash);
    }
}
