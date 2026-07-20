//! Directory listing for the TUI exe picker.
//!
//! Only **directories** and **Windows `.exe` files** are shown so the list stays
//! usable when hunting an installed game under a wineprefix or the host FS.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// One row in the exe browser (directory or `.exe`).
#[derive(Debug, Clone)]
pub struct BrowseEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// True when the path looks like a Windows PE launcher (case-insensitive `.exe`).
pub fn is_windows_exe(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

/// List `dir` contents filtered to directories + `.exe` files (sorted dirs first).
pub fn list_exe_browser(dir: &Path) -> Result<Vec<BrowseEntry>> {
    let mut entries = Vec::new();
    if !dir.is_dir() {
        return Ok(entries);
    }
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        // Skip junk; keep the list focused on navigation + launchables.
        if name == "." || name == ".." {
            continue;
        }
        let ft = ent.file_type()?;
        if ft.is_dir() {
            entries.push(BrowseEntry {
                name,
                path,
                is_dir: true,
            });
        } else if ft.is_file() && is_windows_exe(&path) {
            entries.push(BrowseEntry {
                name,
                path,
                is_dir: false,
            });
        } else if ft.is_symlink() {
            // Follow symlink type so junction-style prefixes still browse.
            if path.is_dir() {
                entries.push(BrowseEntry {
                    name,
                    path,
                    is_dir: true,
                });
            } else if path.is_file() && is_windows_exe(&path) {
                entries.push(BrowseEntry {
                    name,
                    path,
                    is_dir: false,
                });
            }
        }
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

/// Whether navigating to the parent of `cwd` is allowed under `floor`.
///
/// - `floor = Some(p)` (wineprefix mode): stay at or under `p`.
/// - `floor = None` (host FS): allow any parent until filesystem root.
pub fn can_go_up(cwd: &Path, floor: Option<&Path>) -> bool {
    let Some(parent) = cwd.parent() else {
        return false;
    };
    // Path::parent("/") is None on Unix; parent of a single component may be "".
    if parent.as_os_str().is_empty() {
        return false;
    }
    match floor {
        Some(floor) => cwd != floor && cwd.starts_with(floor),
        None => true,
    }
}

/// Prefer `drive_c` inside a wineprefix so users land near Program Files.
pub fn start_dir_for_prefix(prefix: &Path) -> PathBuf {
    let drive_c = prefix.join("drive_c");
    if drive_c.is_dir() {
        drive_c
    } else {
        prefix.to_path_buf()
    }
}

/// Host browse starts at the current exe's parent when it exists, else `$HOME`.
pub fn start_dir_for_filesystem(current_exe: &Path) -> PathBuf {
    if let Some(parent) = current_exe.parent() {
        if parent.is_dir() {
            return parent.to_path_buf();
        }
    }
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn lists_only_dirs_and_exe() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        fs::write(dir.path().join("game.exe"), b"MZ").unwrap();
        fs::write(dir.path().join("Game.EXE"), b"MZ").unwrap();
        fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        fs::write(dir.path().join("data.dll"), b"x").unwrap();

        let entries = list_exe_browser(dir.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"bin"));
        assert!(names.contains(&"game.exe"));
        assert!(names.contains(&"Game.EXE"));
        assert!(!names.iter().any(|n| n.ends_with(".txt") || n.ends_with(".dll")));
        // dirs first
        assert!(entries[0].is_dir);
    }

    #[test]
    fn floor_blocks_escape() {
        let floor = PathBuf::from("/tmp/pfx");
        let inside = floor.join("drive_c").join("Games");
        assert!(can_go_up(&inside, Some(&floor)));
        assert!(!can_go_up(&floor, Some(&floor)));
        assert!(can_go_up(Path::new("/home/u/games"), None));
    }

    #[test]
    fn start_prefers_drive_c() {
        let dir = tempdir().unwrap();
        let pfx = dir.path().join("pfx");
        fs::create_dir_all(pfx.join("drive_c")).unwrap();
        assert_eq!(start_dir_for_prefix(&pfx), pfx.join("drive_c"));
    }
}
