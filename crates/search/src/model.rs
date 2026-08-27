use kaze_shared::FileEntry;
use std::path::{Path, PathBuf};

/// Search model — simple directory walk + substring filter.
/// No indexing, no deps, instant response. Like Finder.
pub struct SearchModel {
    base_path: PathBuf,
}

impl SearchModel {
    pub fn new(base_path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            base_path: base_path.to_path_buf(),
        })
    }

    /// Walk the base directory recursively (max 3 levels deep),
    /// filter by case-insensitive substring match on filename.
    pub fn search(&self, query: &str, limit: usize) -> Vec<FileEntry> {
        if query.is_empty() {
            return vec![];
        }
        let q = query.to_lowercase();
        let mut results = Vec::new();
        walk(&self.base_path, &q, limit, 3, &mut results);
        results
    }

    pub fn wait_for_scan(&self, _timeout: std::time::Duration) {}
}

fn walk(dir: &Path, query: &str, limit: usize, depth: usize, out: &mut Vec<FileEntry>) {
    if out.len() >= limit || depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        if out.len() >= limit {
            return;
        }
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.to_lowercase().contains(query) {
                if let Some(fe) = FileEntry::from_path(path.clone()) {
                    out.push(fe);
                }
            }
        }
        if path.is_dir() {
            walk(&path, query, limit, depth - 1, out);
        }
    }
}
