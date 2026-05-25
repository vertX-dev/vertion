use std::fs;
use std::io;
use std::path::Path;

use crate::config::{detect_comment_style, CommentStyle};
use crate::parser::{detect_marker, MarkerKind};

#[derive(Debug, Clone)]
pub struct Block {
    pub version: String,
    pub tags: Vec<String>,
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
        let m = match detect_marker(line, style) {
            MarkerKind::Versioned(m) | MarkerKind::All(m) => m,
            _ => continue,
        };
        let top_match = stack
            .last()
            .map(|b| b.version == m.version)
            .unwrap_or(false);
        if top_match {
            let mut closed = stack.pop().unwrap();
            closed.end_line = line_no;
            attach(&mut stack, &mut roots, closed);
        } else {
            let depth = stack.len();
            stack.push(Block {
                version: m.version,
                tags: m.tags,
                start_line: line_no,
                end_line: line_no,
                depth,
                children: Vec::new(),
            });
        }
    }
    // Auto-close any leftovers at end-of-file.
    while let Some(mut open) = stack.pop() {
        open.end_line = lines.len();
        attach(&mut stack, &mut roots, open);
    }
    roots
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
    let tag_segment = if show_tags && !blk.tags.is_empty() {
        format!(" tags: {}  ", blk.tags.join(", "))
    } else {
        String::new()
    };
    let label = if blk.version.eq_ignore_ascii_case("ALL") {
        "[ALL]".to_string()
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
    } else {
        blk.version.clone()
    };
    if blk.tags.is_empty() {
        base
    } else {
        format!("{} [{}]", base, blk.tags.join(","))
    }
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
