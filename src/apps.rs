//! Registered applications database (apps.toml).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{is_valid_app_id, Paths, DEFAULT_PREFIX_ID};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppsDb {
    #[serde(default)]
    pub apps: BTreeMap<String, AppEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub exe: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Prefix id relative to prefixes_dir, or absolute path.
    pub prefix: String,
    /// Per-app Proton (directory name, logical `dwproton`, or absolute path).
    /// `None` = inherit global `config.proton.default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proton: Option<String>,
    #[serde(default = "default_game_id")]
    pub game_id: String,
    #[serde(default = "default_store")]
    pub store: String,
    /// Per-app launch locale id (`"ja"`, `"en"`, …). `None` = use global `config.locale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Per-app D3D backend (`"dxvk"` / `"wined3d"`). `None` = `config.launch.graphics`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<String>,
    /// Per-app gamescope wrap. `None` = inherit `[launch.gamescope].enabled`.
    ///
    /// `Some(true)` / `Some(false)` force on/off for this app only (other
    /// gamescope flags still come from global `[launch.gamescope]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamescope: Option<bool>,
    /// When false (default), only global `[launch.env]` is applied; [`Self::env`] is ignored.
    ///
    /// When true, global defaults are applied first, then [`Self::env`] (same keys override).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_custom_env: bool,
    /// Extra launch env when [`Self::use_custom_env`] is true (layered on global defaults).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub icon: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub desktop: bool,
    pub created_at: DateTime<Utc>,
}

fn default_game_id() -> String {
    "umu-default".to_string()
}

fn default_store() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

impl AppsDb {
    pub fn load(paths: &Paths) -> Result<Self> {
        paths.ensure_dirs()?;
        if !paths.apps_file.exists() {
            let db = Self::default();
            db.save(paths)?;
            return Ok(db);
        }
        let text = fs::read_to_string(&paths.apps_file)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        paths.ensure_dirs()?;
        let text = toml::to_string_pretty(self)?;
        fs::write(&paths.apps_file, text)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&AppEntry> {
        self.apps
            .get(id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))
    }

    pub fn insert(&mut self, id: String, entry: AppEntry) -> Result<()> {
        if !is_valid_app_id(&id) {
            return Err(Error::InvalidAppId(id));
        }
        if self.apps.contains_key(&id) {
            return Err(Error::AppExists(id));
        }
        self.apps.insert(id, entry);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<AppEntry> {
        self.apps
            .remove(id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))
    }
}

/// Outcome of [`delete_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletePrefixOutcome {
    /// Directory removed with `remove_dir_all`.
    Removed(PathBuf),
    /// Path already missing (treat as success for cleanup UX).
    AlreadyGone(PathBuf),
}

/// App ids whose resolved prefix path equals `prefix_id`'s resolved path.
///
/// Matches by resolved path so an absolute entry and a relative id that point
/// at the same directory are both counted (shared-bottle safety for delete).
pub fn apps_using_prefix(db: &AppsDb, paths: &Paths, prefix_id: &str) -> Vec<String> {
    let target = paths.prefix_path(prefix_id);
    db.apps
        .iter()
        .filter(|(_, entry)| paths.prefix_path(&entry.prefix) == target)
        .map(|(id, _)| id.clone())
        .collect()
}

/// Delete a managed wineprefix directory under `~/.protoncli`.
///
/// Safety:
/// - always refuses `default` (even with `force`)
/// - refuses absolute paths (no `rm -rf` of arbitrary host paths)
/// - refuses relative ids that fail [`is_valid_app_id`]
/// - if any app still references the prefix and `force` is false → [`Error::PrefixInUse`]
/// - with `force`, the directory is removed but `apps.toml` is left unchanged
///   (apps then show as missing prefix in the TUI)
pub fn delete_prefix(
    paths: &Paths,
    db: &AppsDb,
    id: &str,
    force: bool,
) -> Result<DeletePrefixOutcome> {
    // Absolute paths could point outside our tree; never delete them via this API.
    if Path::new(id).is_absolute() {
        return Err(Error::PrefixAbsolutePath(id.to_string()));
    }
    if id == DEFAULT_PREFIX_ID {
        return Err(Error::PrefixIsDefault);
    }
    if !is_valid_app_id(id) {
        return Err(Error::InvalidAppId(id.to_string()));
    }

    let users = apps_using_prefix(db, paths, id);
    if !users.is_empty() && !force {
        return Err(Error::PrefixInUse {
            id: id.to_string(),
            apps: users.join(", "),
        });
    }

    let path = paths.prefix_path(id);
    if !path.exists() {
        return Ok(DeletePrefixOutcome::AlreadyGone(path));
    }
    fs::remove_dir_all(&path)?;
    Ok(DeletePrefixOutcome::Removed(path))
}

/// Build a new entry after validating the exe path.
pub fn new_entry(
    name: String,
    exe: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    prefix: String,
    proton: Option<String>,
    game_id: Option<String>,
    store: Option<String>,
    desktop: bool,
) -> Result<AppEntry> {
    let exe = if exe.is_absolute() {
        exe
    } else {
        std::env::current_dir()?.join(&exe)
    };
    if !exe.exists() {
        return Err(Error::ExeNotFound(exe));
    }
    let cwd = cwd.or_else(|| exe.parent().map(Path::to_path_buf));
    Ok(AppEntry {
        name,
        exe,
        args,
        cwd,
        prefix,
        proton,
        game_id: game_id.unwrap_or_else(default_game_id),
        store: store.unwrap_or_else(default_store),
        locale: None,
        graphics: None,
        gamescope: None,
        use_custom_env: false,
        env: BTreeMap::new(),
        icon: None,
        desktop,
        created_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_apps_db() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".protoncli");
        let paths = Paths {
            root: root.clone(),
            config_dir: root.clone(),
            data_dir: root.clone(),
            config_file: root.join("config.toml"),
            apps_file: root.join("apps.toml"),
            prefixes_dir: root.join("prefixes"),
            default_prefix: root.join("default"),
            runtimes_dir: root.join("runtimes"),
            icons_dir: root.join("icons"),
            applications_dir: dir.path().join("applications"),
        };
        paths.ensure_dirs().unwrap();

        // Create a fake exe so new_entry accepts it.
        let exe = dir.path().join("game.exe");
        fs::write(&exe, b"MZ").unwrap();

        let mut db = AppsDb::default();
        let entry = new_entry(
            "Test".into(),
            exe,
            vec!["-w".into()],
            None,
            "test".into(),
            None,
            None,
            None,
            true,
        )
        .unwrap();
        db.insert("test".into(), entry).unwrap();
        db.save(&paths).unwrap();

        let loaded = AppsDb::load(&paths).unwrap();
        assert_eq!(loaded.apps["test"].name, "Test");
        assert_eq!(loaded.apps["test"].args, vec!["-w"]);
        // Omit proton at insert → inherit global (not serialized).
        assert_eq!(loaded.apps["test"].proton, None);
    }

    #[test]
    fn proton_override_roundtrip_and_legacy_dwproton() {
        let dir = tempdir().unwrap();
        let paths = test_paths(&dir.path().join(".protoncli"));
        paths.ensure_dirs().unwrap();

        // Explicit per-app pin survives save/load.
        let exe = dir.path().join("game.exe");
        fs::write(&exe, b"MZ").unwrap();
        let mut db = AppsDb::default();
        let mut entry = sample_entry("Pinned", "p", exe);
        entry.proton = Some("GE-Proton9-16".into());
        db.insert("pinned".into(), entry).unwrap();
        db.save(&paths).unwrap();
        let loaded = AppsDb::load(&paths).unwrap();
        assert_eq!(
            loaded.apps["pinned"].proton.as_deref(),
            Some("GE-Proton9-16")
        );

        // Pre-Option apps.toml with proton = "dwproton" still loads as Some.
        fs::write(
            &paths.apps_file,
            r#"
[apps.legacy]
name = "Legacy"
exe = "/tmp/g.exe"
prefix = "default"
proton = "dwproton"
game_id = "umu-default"
store = "none"
desktop = false
created_at = "2024-01-01T00:00:00Z"
"#,
        )
        .unwrap();
        let legacy = AppsDb::load(&paths).unwrap();
        assert_eq!(legacy.apps["legacy"].proton.as_deref(), Some("dwproton"));
    }

    fn test_paths(root: &std::path::Path) -> Paths {
        Paths {
            root: root.to_path_buf(),
            config_dir: root.to_path_buf(),
            data_dir: root.to_path_buf(),
            config_file: root.join("config.toml"),
            apps_file: root.join("apps.toml"),
            prefixes_dir: root.join("prefixes"),
            default_prefix: root.join("default"),
            runtimes_dir: root.join("runtimes"),
            icons_dir: root.join("icons"),
            applications_dir: root.join("applications"),
        }
    }

    fn sample_entry(name: &str, prefix: &str, exe: PathBuf) -> AppEntry {
        new_entry(
            name.into(),
            exe,
            vec![],
            None,
            prefix.into(),
            None,
            None,
            None,
            false,
        )
        .unwrap()
    }

    #[test]
    fn apps_using_prefix_matches_resolved_path() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let exe = dir.path().join("g.exe");
        fs::write(&exe, b"MZ").unwrap();

        let mut db = AppsDb::default();
        db.insert(
            "a".into(),
            sample_entry("A", "shared", exe.clone()),
        )
        .unwrap();
        db.insert(
            "b".into(),
            sample_entry("B", "other", exe.clone()),
        )
        .unwrap();
        // Absolute path that resolves to the same bottle as relative id "shared".
        let abs = paths.prefix_path("shared");
        db.insert(
            "c".into(),
            sample_entry("C", abs.to_str().unwrap(), exe),
        )
        .unwrap();

        let users = apps_using_prefix(&db, &paths, "shared");
        assert_eq!(users, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn delete_prefix_refuses_default() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let db = AppsDb::default();
        assert!(matches!(
            delete_prefix(&paths, &db, "default", true),
            Err(Error::PrefixIsDefault)
        ));
    }

    #[test]
    fn delete_prefix_refuses_absolute_path() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let db = AppsDb::default();
        let abs = dir.path().join("outside").to_string_lossy().into_owned();
        assert!(matches!(
            delete_prefix(&paths, &db, &abs, true),
            Err(Error::PrefixAbsolutePath(_))
        ));
    }

    #[test]
    fn delete_prefix_refuses_in_use_without_force() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let pfx = paths.prefix_path("in-use");
        fs::create_dir_all(&pfx).unwrap();
        let exe = dir.path().join("g.exe");
        fs::write(&exe, b"MZ").unwrap();

        let mut db = AppsDb::default();
        db.insert("game".into(), sample_entry("Game", "in-use", exe))
            .unwrap();

        let err = delete_prefix(&paths, &db, "in-use", false).unwrap_err();
        match err {
            Error::PrefixInUse { id, apps } => {
                assert_eq!(id, "in-use");
                assert_eq!(apps, "game");
            }
            other => panic!("expected PrefixInUse, got {other:?}"),
        }
        assert!(pfx.is_dir());
    }

    #[test]
    fn delete_prefix_force_removes_dir_keeps_apps() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let pfx = paths.prefix_path("forced");
        fs::create_dir_all(pfx.join("pfx")).unwrap();
        let exe = dir.path().join("g.exe");
        fs::write(&exe, b"MZ").unwrap();

        let mut db = AppsDb::default();
        db.insert("game".into(), sample_entry("Game", "forced", exe))
            .unwrap();

        let out = delete_prefix(&paths, &db, "forced", true).unwrap();
        assert_eq!(out, DeletePrefixOutcome::Removed(pfx.clone()));
        assert!(!pfx.exists());
        // apps.toml references are intentionally left for missing-prefix UX
        assert!(db.apps.contains_key("game"));
    }

    #[test]
    fn delete_prefix_orphan_and_already_gone() {
        let dir = tempdir().unwrap();
        let paths = test_paths(dir.path());
        paths.ensure_dirs().unwrap();
        let pfx = paths.prefix_path("orphan");
        fs::create_dir_all(&pfx).unwrap();
        let db = AppsDb::default();

        let out = delete_prefix(&paths, &db, "orphan", false).unwrap();
        assert_eq!(out, DeletePrefixOutcome::Removed(pfx.clone()));
        assert!(!pfx.exists());

        let out2 = delete_prefix(&paths, &db, "orphan", false).unwrap();
        assert_eq!(out2, DeletePrefixOutcome::AlreadyGone(pfx));
    }
}
