use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use crate::config::detect_comment_style;
use crate::inspect::{collect_blocks, Block};

#[derive(Debug, Default, Serialize)]
pub struct StatsSummary {
    pub files_scanned: usize,
    pub files_with_markers: usize,
    pub total_blocks: usize,
    pub tagged_blocks: usize,
    pub deepest_nesting: usize,
    pub average_nesting: f64,
    pub version_distribution: BTreeMap<String, usize>,
    pub tag_distribution: BTreeMap<String, usize>,
    pub top_files_by_blocks: Vec<(PathBuf, usize)>,
}

pub fn gather(root: &Path, ignore: &[PathBuf]) -> io::Result<StatsSummary> {
    let mut s = StatsSummary::default();
    let ignore_abs: Vec<PathBuf> = ignore.iter().map(|p| absolute(p.as_path())).collect();
    let mut depth_sum: usize = 0;
    let mut per_file: Vec<(PathBuf, usize)> = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let abs = absolute(path);
        if ignore_abs.iter().any(|ig| abs.starts_with(ig)) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        s.files_scanned += 1;
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
        let style = detect_comment_style(ext);
        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let roots = collect_blocks(&lines, style);
        if roots.is_empty() {
            continue;
        }
        s.files_with_markers += 1;
        let mut file_count = 0usize;
        for r in &roots {
            walk(r, &mut s, &mut depth_sum, &mut file_count);
        }
        per_file.push((path.to_path_buf(), file_count));
    }

    if s.total_blocks > 0 {
        s.average_nesting = depth_sum as f64 / s.total_blocks as f64;
    }
    per_file.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    s.top_files_by_blocks = per_file.into_iter().take(5).collect();
    Ok(s)
}

fn walk(blk: &Block, s: &mut StatsSummary, depth_sum: &mut usize, file_count: &mut usize) {
    s.total_blocks += 1;
    *file_count += 1;
    *depth_sum += blk.depth;
    if blk.depth + 1 > s.deepest_nesting {
        s.deepest_nesting = blk.depth + 1;
    }
    if !blk.tags.is_empty() {
        s.tagged_blocks += 1;
        for t in &blk.tags {
            *s.tag_distribution.entry(t.clone()).or_insert(0) += 1;
        }
    }
    *s.version_distribution
        .entry(blk.version.clone())
        .or_insert(0) += 1;
    for c in &blk.children {
        walk(c, s, depth_sum, file_count);
    }
}

pub fn render_table(s: &StatsSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!("Files scanned       : {}\n", s.files_scanned));
    out.push_str(&format!("Files with markers  : {}\n", s.files_with_markers));
    out.push_str(&format!("Total blocks        : {}\n", s.total_blocks));
    out.push_str(&format!("Tagged blocks       : {}\n", s.tagged_blocks));
    out.push_str(&format!("Deepest nesting     : {}\n", s.deepest_nesting));
    out.push_str(&format!("Average nesting     : {:.2}\n", s.average_nesting));
    if !s.version_distribution.is_empty() {
        out.push_str("\nVersion distribution:\n");
        for (v, c) in &s.version_distribution {
            out.push_str(&format!("  {:<12} {}\n", v, c));
        }
    }
    if !s.tag_distribution.is_empty() {
        out.push_str("\nTag distribution:\n");
        for (t, c) in &s.tag_distribution {
            out.push_str(&format!("  {:<12} {}\n", t, c));
        }
    }
    if !s.top_files_by_blocks.is_empty() {
        out.push_str("\nTop files by block count:\n");
        for (p, c) in &s.top_files_by_blocks {
            out.push_str(&format!("  {:<40} {}\n", p.display(), c));
        }
    }
    out
}

pub fn render_json(s: &StatsSummary) -> String {
    serde_json::to_string_pretty(s).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}
