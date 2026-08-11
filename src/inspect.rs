use std::fs;
use std::io;
use std::path::Path;

use crate::config::{detect_comment_style, CommentStyle};
use crate::parser::{detect_marker, MarkerCondition, MarkerKind};

#[derive(Debug, Clone)]
pub struct Block {
    pub version: String,
    /// Upper bound for range markers (block or inline). `None` for single-version markers.
    pub to: Option<String>,
    /// True for inline range markers (`//version 1.3 2.0` with no `*`). Single-line entries.
    pub inline: bool,
    pub tags: Vec<String>,
    /// Conditions attached to this block's tags.
    pub conditions: Vec<MarkerCondition>,
    pub start_line: usize,
    pub end_line: usize,
    pub depth: usize,
    pub children: Vec<Block>,
}

pub fn collect_blocks_from_file(path: &Path) -> io::Result<Vec<Block>> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let style = detect_comment_style(ext);
    let text = fs::read_to_string(path)?;
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Ok(collect_blocks(&lines, style))
}

pub fn collect_blocks(lines: &[String], style: CommentStyle) -> Vec<Block> {
    let mut stack: Vec<Block> = Vec::new();
    let mut roots: Vec<Block> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        match detect_marker(line, style) {
            MarkerKind::Versioned(m)
            | MarkerKind::All(m)
            | MarkerKind::Exclude(m)
            | MarkerKind::TagOnly(m) => {
                // Tag-only markers pair on a synthetic `[tags]` label, so blocks
                // with different tag sets stay distinct.
                let label = m.pair_key();
                let top_match = stack
                    .last()
                    .map(|b| b.version == label && b.to == m.to)
                    .unwrap_or(false);
                if top_match {
                    let mut closed = stack.pop().unwrap();
                    closed.end_line = line_no;
                    attach(&mut stack, &mut roots, closed);
                } else {
                    let depth = stack.len();
                    stack.push(Block {
                        version: label,
                        to: m.to,
                        inline: false,
                        tags: m.tags,
                        conditions: m.conditions,
                        start_line: line_no,
                        end_line: line_no,
                        depth,
                        children: Vec::new(),
                    });
                }
            }
            MarkerKind::InlineRange(m) => {
                // Inline range: a leaf node attached to whatever's open above it.
                let depth = stack.len();
                let blk = Block {
                    version: m.version,
                    to: m.to,
                    inline: true,
                    tags: m.tags,
                    conditions: m.conditions,
                    start_line: line_no,
                    end_line: line_no,
                    depth,
                    children: Vec::new(),
                };
                attach(&mut stack, &mut roots, blk);
            }
            _ => continue,
        }
    }
    // Auto-close any leftovers at end-of-file.
    while let Some(mut open) = stack.pop() {
        open.end_line = lines.len();
        attach(&mut stack, &mut roots, open);
    }
    roots
}

/// Render a block's conditions in source form (`cond`, `!cond`).
fn display_conditions(blk: &Block, sep: &str) -> String {
    blk.conditions
        .iter()
        .map(|c| c.display())
        .collect::<Vec<_>>()
        .join(sep)
}

fn attach(stack: &mut [Block], roots: &mut Vec<Block>, blk: Block) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(blk);
    } else {
        roots.push(blk);
    }
}

/// Render a `--show` listing: flat indented list with line ranges (and optional tags).
pub fn render_show(blocks: &[Block], show_tags: bool, total_lines: usize) -> String {
    let mut out = String::new();
    // Base region 0: from line 1 up to start of first top-level block (or EOF).
    let mut cursor = 1usize;
    for blk in blocks {
        if blk.start_line > cursor {
            out.push_str(&format!(
                "[base]          lines {}-{}\n",
                cursor,
                blk.start_line - 1
            ));
        }
        render_block(blk, show_tags, &mut out);
        cursor = blk.end_line + 1;
    }
    if cursor <= total_lines {
        out.push_str(&format!(
            "[base]          lines {}-{}\n",
            cursor, total_lines
        ));
    }
    out
}

fn render_block(blk: &Block, show_tags: bool, out: &mut String) {
    let indent = "  ".repeat(blk.depth);
    let mut tag_segment = if show_tags && !blk.tags.is_empty() {
        format!(" tags: {}  ", blk.tags.join(", "))
    } else {
        String::new()
    };
    if show_tags && !blk.conditions.is_empty() {
        tag_segment.push_str(&format!(" if: {}  ", display_conditions(blk, ", ")));
    }
    if blk.inline {
        // Inline range: single-line entry.
        let to = blk.to.as_deref().unwrap_or("?");
        out.push_str(&format!(
            "{}[inline {} → {}]{} line {}\n",
            indent, blk.version, to, tag_segment, blk.start_line
        ));
        return;
    }
    let label = if blk.version.eq_ignore_ascii_case("ALL") {
        "[ALL]".to_string()
    } else if blk.version.eq_ignore_ascii_case("EXC") {
        "[EXC]".to_string()
    } else if blk.version.starts_with('[') {
        // Tag-only block: `version` already holds the synthetic `[tags]` label.
        format!("[tags {}]", blk.version.trim_matches(['[', ']']))
    } else if let Some(to) = &blk.to {
        format!("[version {} → {}]", blk.version, to)
    } else {
        format!("[version {}]", blk.version)
    };
    out.push_str(&format!(
        "{}{}{} lines {}-{}\n",
        indent, label, tag_segment, blk.start_line, blk.end_line
    ));
    for c in &blk.children {
        render_block(c, show_tags, out);
    }
}

/// Render a `--graph` tree using ├── / └── connectors.
pub fn render_graph(blocks: &[Block]) -> String {
    let mut out = String::new();
    let n = blocks.len();
    for (i, blk) in blocks.iter().enumerate() {
        let last = i + 1 == n;
        render_graph_node(blk, "", last, &mut out);
    }
    out
}

fn render_graph_node(blk: &Block, prefix: &str, is_last: bool, out: &mut String) {
    let branch = if is_last { "└── " } else { "├── " };
    let label = format_graph_label(blk);
    out.push_str(&format!("{}{}{}\n", prefix, branch, label));
    let child_prefix = if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };
    let n = blk.children.len();
    for (i, c) in blk.children.iter().enumerate() {
        render_graph_node(c, &child_prefix, i + 1 == n, out);
    }
}

fn format_graph_label(blk: &Block) -> String {
    let base = if blk.version.eq_ignore_ascii_case("ALL") {
        "ALL".to_string()
    } else if let Some(to) = &blk.to {
        let prefix = if blk.inline { "inline " } else { "" };
        format!("{}{} → {}", prefix, blk.version, to)
    } else {
        blk.version.clone()
    };
    // Tag-only blocks already carry their tags in `version` (as `[a,b]`).
    let mut out = if blk.tags.is_empty() || blk.version.starts_with('[') {
        base
    } else {
        format!("{} [{}]", base, blk.tags.join(","))
    };
    if !blk.conditions.is_empty() {
        out.push_str(&format!(" if {}", display_conditions(blk, ",")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn collects_flat_blocks() {
        let src = "a\n//version 1.1 *\nb\n//version 1.1 *\nc";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].version, "1.1");
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 4);
    }

    #[test]
    fn collects_nested_blocks_with_tags() {
        let src = "\
a
//version 1.1 *
b
//version 1.2 [beta] *
c
//version 1.2 *
d
//version 1.1 *
e
//version 2.0 [inventory] *
f
//version 2.0 *
g";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].version, "1.1");
        assert_eq!(blocks[0].children.len(), 1);
        assert_eq!(blocks[0].children[0].version, "1.2");
        assert_eq!(blocks[0].children[0].tags, vec!["beta".to_string()]);
        assert_eq!(blocks[1].version, "2.0");
        assert_eq!(blocks[1].tags, vec!["inventory".to_string()]);
    }

    #[test]
    fn show_renders_base_and_blocks() {
        let src = "\
a
//version 1.1 *
b
//version 1.1 *
c";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        let out = render_show(&blocks, false, 5);
        assert!(out.contains("[base]"));
        assert!(out.contains("[version 1.1]"));
    }

    #[test]
    fn show_renders_range_block_with_arrow() {
        let src = "//version 1.3 2.0 *\nbody\n//version 1.3 2.0 *\n";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        let out = render_show(&blocks, false, 3);
        assert!(out.contains("[version 1.3 → 2.0]"));
    }

    #[test]
    fn show_renders_inline_range_as_single_line() {
        let src = "x\n//version 1.3 2.0\ntarget\ny\n";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        let out = render_show(&blocks, false, 4);
        assert!(out.contains("[inline 1.3 → 2.0]"));
        assert!(out.contains("line 2"));
    }

    #[test]
    fn graph_renders_range_with_arrow() {
        let src = "//version 1.3 2.0 [beta] *\nx\n//version 1.3 2.0 [beta] *\n";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        let out = render_graph(&blocks);
        assert!(out.contains("1.3 → 2.0 [beta]"));
    }

    #[test]
    fn graph_uses_tree_connectors() {
        let src = "\
//version 1.1 *
//version 1.2 [beta] *
//version 1.2 *
//version 1.1 *
//version 2.0 [inventory] *
//version 2.0 *";
        let blocks = collect_blocks(&lines(src), CommentStyle::DoubleSlash);
        let out = render_graph(&blocks);
        assert!(out.contains("├──") || out.contains("└──"));
        assert!(out.contains("1.2 [beta]"));
        assert!(out.contains("2.0 [inventory]"));
    }
}
