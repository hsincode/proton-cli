//! Registered applications database (apps.toml).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{is_valid_app_id, Paths};

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
    #[serde(default = "default_proton")]
    pub proton: String,
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
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub icon: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub desktop: bool,
    pub created_at: DateTime<Utc>,
}

fn default_proton() -> String {
    "dwproton".to_string()
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

/// Build a new entry after validating the exe path.
pub fn new_entry(
    name: String,
    exe: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    prefix: String,
    proton: String,
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
            "dwproton".into(),
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
    }
}
