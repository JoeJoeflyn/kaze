use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub has_children: bool,
    pub cached_size_label: String,
    pub cached_date_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}

fn format_size_label(size: u64, is_dir: bool) -> String {
    if is_dir {
        return "--".to_string();
    }
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.0} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

fn format_date_label(modified: Option<SystemTime>) -> String {
    let Some(time) = modified else {
        return "--".to_string();
    };
    let duration = SystemTime::now()
        .duration_since(time)
        .unwrap_or_default();
    let days = duration.as_secs() / 86400;
    if days == 0 {
        "Today".to_string()
    } else if days == 1 {
        "Yesterday".to_string()
    } else if days < 7 {
        format!("{} days ago", days)
    } else if days < 30 {
        format!("{} weeks ago", days / 7)
    } else if days < 365 {
        format!("{} months ago", days / 30)
    } else {
        format!("{} years ago", days / 365)
    }
}

impl FileEntry {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let metadata = std::fs::symlink_metadata(&path).ok()?;
        let kind = if metadata.file_type().is_symlink() {
            FileKind::Symlink
        } else if metadata.file_type().is_dir() {
            FileKind::Directory
        } else {
            FileKind::File
        };

        let size = metadata.len();
        let modified = metadata.modified().ok();
        let is_dir = kind == FileKind::Directory;

        Some(Self {
            has_children: is_dir
                && std::fs::read_dir(&path)
                    .map(|rd| rd.filter_map(|e| e.ok()).next().is_some())
                    .unwrap_or(false),
            cached_size_label: format_size_label(size, is_dir),
            cached_date_label: format_date_label(modified),
            path,
            name,
            kind,
            size,
            modified,
        })
    }

    pub fn is_dir(&self) -> bool {
        self.kind == FileKind::Directory
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            FileKind::Directory => "Folder",
            FileKind::Symlink => "Alias",
            FileKind::File => "Document",
        }
    }

    pub fn size_label(&self) -> &str {
        &self.cached_size_label
    }

    pub fn date_label(&self) -> &str {
        &self.cached_date_label
    }
}
