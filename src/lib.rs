//! Library surface for embedding Vertion's version-marker filter in other
//! tools (e.g. wikidata's `--for-version`). The `vertion` binary
//! (`src/main.rs`) is a separate crate root and is unaffected by this file.
//!
//! The reusable pieces are pure functions:
//! - [`filter::parse_filter`] — parse `["1.2"]` / `["1.1","1.2"]` into a [`filter::FilterMode`]
//! - [`parser::process_file`] — strip version blocks that don't pass the filter
//! - [`config::detect_comment_style`] / [`config::CommentStyle`] — `//` vs `#` by extension

pub mod config;
pub mod filter;
pub mod parser;
