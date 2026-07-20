//! Global config.toml loading and defaults.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub proton: ProtonConfig,
    pub umu: UmuConfig,
    pub desktop: DesktopConfig,
    /// Default wineprefix id used when `add` / `run` omit `--prefix`.
    /// `"default"` resolves to `~/.protoncli/default`.
    pub default_prefix: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            proton: ProtonConfig::default(),
            umu: UmuConfig::default(),
            desktop: DesktopConfig::default(),
            default_prefix: crate::paths::DEFAULT_PREFIX_ID.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtonConfig {
    /// Logical default: "dwproton" resolves to latest managed/steam install.
    pub default: String,
    /// Where to look first: managed | steam | path
    pub prefer: String,
    /// Optional absolute override for PROTONPATH.
    pub explicit_path: Option<PathBuf>,
}

impl Default for ProtonConfig {
    fn default() -> Self {
        Self {
            default: "dwproton".to_string(),
            prefer: "managed".to_string(),
            explicit_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UmuConfig {
    /// Optional absolute path or name of umu-run.
    pub command: Option<String>,
}

impl Default for UmuConfig {
    fn default() -> Self {
        Self { command: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    pub create_on_add: bool,
    pub categories: Vec<String>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            create_on_add: true,
            categories: vec!["Game".to_string()],
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        paths.ensure_dirs()?;
        if !paths.config_file.exists() {
            let cfg = Self::default();
            cfg.save(paths)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(&paths.config_file)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        paths.ensure_dirs()?;
        let text = toml::to_string_pretty(self)?;
        fs::write(&paths.config_file, text)?;
        Ok(())
    }
}
