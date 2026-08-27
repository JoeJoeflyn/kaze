use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Move a file or directory to the system trash.
/// 1. Tries `gio trash` (the native GNOME / FreeDesktop standard tool).
/// 2. Falls back to manual FreeDesktop trash specification (~/.local/share/Trash).
pub fn move_to_trash(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    // 1. Try native Linux gio trash first
    if let Ok(status) = Command::new("gio").arg("trash").arg(path).status() {
        if status.success() && !path.exists() {
            return Ok(());
        }
    }

    // 2. Fallback to manual FreeDesktop Trash specification
    let trash_base = dirs::trash_base_dir().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(home).join(".local/share/Trash")
    });

    let trash_files = trash_base.join("files");
    let trash_info = trash_base.join("info");

    std::fs::create_dir_all(&trash_files)?;
    std::fs::create_dir_all(&trash_info)?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    let mut dest_name = file_name.clone();
    let mut dest_file = trash_files.join(&dest_name);

    // Handle name collisions by appending a number
    if dest_file.exists() {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        for i in 1..10000 {
            let candidate_name = format!("{} ({}){}", stem, i, ext);
            let candidate_file = trash_files.join(&candidate_name);
            if !candidate_file.exists() {
                dest_name = candidate_name;
                dest_file = candidate_file;
                break;
            }
        }
    }

    // Write .trashinfo metadata
    let info_file = trash_info.join(format!("{}.trashinfo", dest_name));
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "0".to_string(),
    };
    let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let trashinfo_content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        abs_path.to_string_lossy(),
        now
    );
    let _ = std::fs::write(&info_file, trashinfo_content);

    // Attempt rename, fallback to copy+remove if cross-device or permission boundary
    if let Err(_) = std::fs::rename(path, &dest_file) {
        if path.is_dir() {
            copy_dir_recursive(path, &dest_file)?;
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::copy(path, &dest_file)?;
            std::fs::remove_file(path)?;
        }
    }

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

mod dirs {
    use std::path::PathBuf;

    pub fn trash_base_dir() -> Option<PathBuf> {
        std::env::var("XDG_DATA_HOME")
            .ok()
            .map(|d| PathBuf::from(d).join("Trash"))
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/share/Trash"))
            })
    }

    pub fn trash_dir() -> Option<PathBuf> {
        trash_base_dir().map(|b| b.join("files"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_move_to_trash_basic() {
        let tmp = std::env::temp_dir().join("kaze_trash_test_basic");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let file = tmp.join("test_file.txt");
        fs::write(&file, "hello").unwrap();

        let result = move_to_trash(&file);
        assert!(result.is_ok(), "move_to_trash failed: {:?}", result);
        assert!(!file.exists(), "source file still exists after trash");
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_move_to_trash_name_collision() {
        let tmp = std::env::temp_dir().join("kaze_trash_test_collision");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let file1 = tmp.join("dup.txt");
        fs::write(&file1, "first").unwrap();
        assert!(move_to_trash(&file1).is_ok());
        assert!(!file1.exists());

        let file2 = tmp.join("dup.txt");
        fs::write(&file2, "second").unwrap();
        assert!(move_to_trash(&file2).is_ok());
        assert!(!file2.exists());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_move_to_trash_directory() {
        let tmp = std::env::temp_dir().join("kaze_trash_test_dir");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let dir = tmp.join("my_folder");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("inside.txt"), "content").unwrap();

        assert!(move_to_trash(&dir).is_ok());
        assert!(!dir.exists(), "source dir still exists after trash");

        fs::remove_dir_all(&tmp).unwrap();
    }
}
