#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    DoubleSlash,
    Hash,
}

impl CommentStyle {
    pub fn prefix(&self) -> &'static str {
        match self {
            CommentStyle::DoubleSlash => "//",
            CommentStyle::Hash => "#",
        }
    }
}

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
