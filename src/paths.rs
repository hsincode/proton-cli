//! Fixed layout under `~/.protoncli` (plus XDG applications for shortcuts).
//!
//! ```text
//! ~/.protoncli/
//!   config.toml
//!   apps.toml
//!   default/              # default WINEPREFIX (prefix id "default")
//!   prefixes/<name>/      # named prefixes
//!   runtimes/             # managed Proton (dwproton, …)
//!   icons/
//! ~/.local/share/applications/
//!   proton-cli-<id>.desktop
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::error::{Error, Result};

/// Logical id of the shared default wineprefix (`~/.protoncli/default`).
pub const DEFAULT_PREFIX_ID: &str = "default";

/// Root directory name under `$HOME`.
pub const ROOT_DIR_NAME: &str = ".protoncli";

/// Resolved filesystem layout for one proton-cli instance.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `~/.protoncli`
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub apps_file: PathBuf,
    /// Parent of named prefixes: `~/.protoncli/prefixes`
    pub prefixes_dir: PathBuf,
    /// Default wineprefix: `~/.protoncli/default`
    pub default_prefix: PathBuf,
    pub runtimes_dir: PathBuf,
    pub icons_dir: PathBuf,
    pub applications_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let home = BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .ok_or_else(|| {
                Error::Other(anyhow::anyhow!(
                    "could not resolve home directory (HOME missing?)"
                ))
            })?;

        let root = home.join(ROOT_DIR_NAME);
        // Config and data live together under ~/.protoncli for a single fixed root.
        let config_dir = root.clone();
        let data_dir = root.clone();

        let applications_dir = home.join(".local/share/applications");

        Ok(Self {
            config_file: root.join("config.toml"),
            apps_file: root.join("apps.toml"),
            prefixes_dir: root.join("prefixes"),
            default_prefix: root.join(DEFAULT_PREFIX_ID),
            runtimes_dir: root.join("runtimes"),
            icons_dir: root.join("icons"),
            applications_dir,
            config_dir,
            data_dir,
            root,
        })
    }

    /// Create config/data directories if they do not exist yet.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.root,
            &self.prefixes_dir,
            &self.runtimes_dir,
            &self.icons_dir,
            &self.applications_dir,
            &self.default_prefix,
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Resolve a prefix id or absolute path to a full WINEPREFIX path.
    ///
    /// - absolute path → as-is
    /// - `"default"` → `~/.protoncli/default`
    /// - other relative id → `~/.protoncli/prefixes/<id>`
    pub fn prefix_path(&self, prefix_id: &str) -> PathBuf {
        let p = Path::new(prefix_id);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        if prefix_id == DEFAULT_PREFIX_ID {
            return self.default_prefix.clone();
        }
        self.prefixes_dir.join(prefix_id)
    }

    /// List known prefix ids (default + subdirs of prefixes/).
    pub fn list_prefix_ids(&self) -> Result<Vec<String>> {
        let mut ids = vec![DEFAULT_PREFIX_ID.to_string()];
        if self.prefixes_dir.is_dir() {
            let mut named = Vec::new();
            for entry in fs::read_dir(&self.prefixes_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if is_valid_app_id(name) {
                            named.push(name.to_string());
                        }
                    }
                }
            }
            named.sort();
            ids.extend(named);
        }
        Ok(ids)
    }

    pub fn desktop_file(&self, app_id: &str) -> PathBuf {
        self.applications_dir
            .join(format!("proton-cli-{app_id}.desktop"))
    }
}

/// App / prefix ids must be safe for filenames, .desktop names, and CLI args.
pub fn is_valid_app_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    let mut chars = id.chars();
    // First char: letter or digit
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Derive a slug from a display name (lossy).
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        let lower = c.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ids() {
        assert!(is_valid_app_id("my-game"));
        assert!(is_valid_app_id("a"));
        assert!(is_valid_app_id("game2"));
        assert!(is_valid_app_id("default"));
        assert!(!is_valid_app_id(""));
        assert!(!is_valid_app_id("My-Game"));
        assert!(!is_valid_app_id("-bad"));
        assert!(!is_valid_app_id("has space"));
        assert!(!is_valid_app_id("has_under"));
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool Game!"), "my-cool-game");
        assert_eq!(slugify("  "), "app");
    }

    #[test]
    fn prefix_path_default_and_named() {
        let paths = Paths {
            root: PathBuf::from("/home/u/.protoncli"),
            config_dir: PathBuf::from("/home/u/.protoncli"),
            data_dir: PathBuf::from("/home/u/.protoncli"),
            config_file: PathBuf::from("/home/u/.protoncli/config.toml"),
            apps_file: PathBuf::from("/home/u/.protoncli/apps.toml"),
            prefixes_dir: PathBuf::from("/home/u/.protoncli/prefixes"),
            default_prefix: PathBuf::from("/home/u/.protoncli/default"),
            runtimes_dir: PathBuf::from("/home/u/.protoncli/runtimes"),
            icons_dir: PathBuf::from("/home/u/.protoncli/icons"),
            applications_dir: PathBuf::from("/home/u/.local/share/applications"),
        };
        assert_eq!(
            paths.prefix_path("default"),
            PathBuf::from("/home/u/.protoncli/default")
        );
        assert_eq!(
            paths.prefix_path("my-game"),
            PathBuf::from("/home/u/.protoncli/prefixes/my-game")
        );
        assert_eq!(
            paths.prefix_path("/opt/wine/pfx"),
            PathBuf::from("/opt/wine/pfx")
        );
    }
}
