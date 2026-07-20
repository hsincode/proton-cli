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
    /// Launch defaults shared by every app (locale is separate for first-run UX).
    pub launch: LaunchConfig,
    /// Default wineprefix id used when `add` / `run` omit `--prefix`.
    /// `"default"` resolves to `~/.protoncli/default`.
    pub default_prefix: String,
    /// Global launch locale id from the catalog (`"ja"`, `"en"`, …).
    ///
    /// `None` means not configured yet — the TUI shows a first-run picker.
    /// Applied as `LANG` / `LC_*` at launch (umu generates the locale in-container;
    /// host `locale-gen` is not required). Per-app `env` in `apps.toml` overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            proton: ProtonConfig::default(),
            umu: UmuConfig::default(),
            desktop: DesktopConfig::default(),
            launch: LaunchConfig::default(),
            default_prefix: crate::paths::DEFAULT_PREFIX_ID.to_string(),
            locale: None,
        }
    }
}

/// Global launch behaviour (Steam/Lutris-like defaults).
///
/// Per-app overrides live on [`crate::apps::AppEntry`] (`locale`, `graphics`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchConfig {
    /// When true (default), strip host `VK_ICD_FILENAMES` / `VK_DRIVER_FILES` at
    /// launch so 32-bit and 64-bit Proton games both find GPU drivers.
    ///
    /// Host profiles sometimes pin only the x86_64 ICD (breaks PE32 DXVK).
    /// Steam / Lutris isolate this; we mirror that unless the user opts out.
    pub sanitize_vulkan: bool,
    /// Default D3D translation: `"dxvk"` (Vulkan) or `"wined3d"` (OpenGL).
    /// Per-app `graphics` overrides this.
    pub graphics: String,
    /// How much umu / Proton / Wine chatter to show on the terminal.
    ///
    /// - `"quiet"` — hide child stdout/stderr (daily play; use `normal` to debug)
    /// - `"normal"` — inherit streams (default; current pre-setting behaviour)
    /// - `"verbose"` — inherit + extra protoncli launch diagnostics
    pub log_level: String,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            sanitize_vulkan: true,
            graphics: "dxvk".into(),
            // Default stays "noisy" so upgrades and first-time installs keep
            // failure output visible; quiet is an explicit player choice.
            log_level: "normal".into(),
        }
    }
}

/// Known graphics backend ids accepted in config / apps / CLI.
pub fn is_valid_graphics(id: &str) -> bool {
    matches!(id, "dxvk" | "wined3d")
}

/// Normalize a graphics id; unknown values fall back to `dxvk`.
pub fn normalize_graphics(id: &str) -> &'static str {
    match id {
        "wined3d" => "wined3d",
        _ => "dxvk",
    }
}

/// Known launch log-level ids accepted in config / CLI.
pub fn is_valid_log_level(id: &str) -> bool {
    matches!(id, "quiet" | "normal" | "verbose")
}

/// Normalize a log-level id; unknown values fall back to `normal`.
pub fn normalize_log_level(id: &str) -> &'static str {
    match id {
        "quiet" => "quiet",
        "verbose" => "verbose",
        _ => "normal",
    }
}

/// Cycle quiet → normal → verbose → quiet (TUI Enter on the log-level row).
pub fn next_log_level(id: &str) -> &'static str {
    match normalize_log_level(id) {
        "quiet" => "normal",
        "normal" => "verbose",
        _ => "quiet",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use tempfile::tempdir;

    #[test]
    fn locale_roundtrip_and_legacy_missing() {
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

        // Pre-locale config still loads with locale = None (first-run TUI).
        fs::write(
            &paths.config_file,
            r#"
default_prefix = "default"
[proton]
default = "dwproton"
prefer = "managed"
[umu]
[desktop]
create_on_add = true
categories = ["Game"]
"#,
        )
        .unwrap();
        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.locale, None);

        let mut cfg = cfg;
        cfg.locale = Some("ja".into());
        cfg.launch.graphics = "wined3d".into();
        cfg.launch.sanitize_vulkan = false;
        cfg.launch.log_level = "quiet".into();
        cfg.save(&paths).unwrap();
        let loaded = Config::load(&paths).unwrap();
        assert_eq!(loaded.locale.as_deref(), Some("ja"));
        assert_eq!(loaded.launch.graphics, "wined3d");
        assert!(!loaded.launch.sanitize_vulkan);
        assert_eq!(loaded.launch.log_level, "quiet");

        // Legacy config without [launch] still gets Steam-like defaults.
        fs::write(
            &paths.config_file,
            r#"
default_prefix = "default"
locale = "en"
[proton]
default = "dwproton"
prefer = "managed"
[umu]
[desktop]
create_on_add = true
categories = ["Game"]
"#,
        )
        .unwrap();
        let legacy = Config::load(&paths).unwrap();
        assert!(legacy.launch.sanitize_vulkan);
        assert_eq!(legacy.launch.graphics, "dxvk");
        assert_eq!(legacy.launch.log_level, "normal");
    }

    #[test]
    fn graphics_ids() {
        assert!(is_valid_graphics("dxvk"));
        assert!(is_valid_graphics("wined3d"));
        assert!(!is_valid_graphics("vkd3d"));
        assert_eq!(normalize_graphics("wined3d"), "wined3d");
        assert_eq!(normalize_graphics("nope"), "dxvk");
    }

    #[test]
    fn log_level_ids() {
        assert!(is_valid_log_level("quiet"));
        assert!(is_valid_log_level("normal"));
        assert!(is_valid_log_level("verbose"));
        assert!(!is_valid_log_level("debug"));
        assert_eq!(normalize_log_level("quiet"), "quiet");
        assert_eq!(normalize_log_level("VERBOSE"), "normal"); // case-sensitive; unknown → normal
        assert_eq!(normalize_log_level("nope"), "normal");
        assert_eq!(next_log_level("quiet"), "normal");
        assert_eq!(next_log_level("normal"), "verbose");
        assert_eq!(next_log_level("verbose"), "quiet");
    }
}
