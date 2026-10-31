//! Global config.toml loading and defaults.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::Paths;

/// Built-in launch env applied when `[launch.env]` is absent from config.toml.
///
/// - `DXVK_HUD=0` — hide dwproton/DXVK "Compiling shaders..." (Proton setdefaults `compiler`)
/// - `STEAMOS=1` / `UMU_USE_STEAM=1` — Steam Deck–like / Steam-runtime umu paths many titles expect
pub fn builtin_launch_env() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("DXVK_HUD".into(), "0".into());
    m.insert("STEAMOS".into(), "1".into());
    m.insert("UMU_USE_STEAM".into(), "1".into());
    m
}

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
/// Per-app overrides live on [`crate::apps::AppEntry`] (`proton`, `locale`,
/// `graphics`, `gamescope`, env).
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
    /// Optional [gamescope](https://github.com/ValveSoftware/gamescope) wrapper.
    ///
    /// Default is off. When enabled, launches become
    /// `gamescope [flags…] -- <umu|steam+proton> …` so exclusive-fullscreen titles
    /// get a single nested display (multi-monitor friendly; real FS still works
    /// via `-f` / `--force-windows-fullscreen`).
    pub gamescope: GamescopeConfig,
    /// Default environment variables for every launch.
    ///
    /// Applied after locale / graphics env; per-app custom env can override keys.
    /// Missing from config.toml → [`builtin_launch_env`]. Explicit empty table stays empty.
    #[serde(default = "builtin_launch_env")]
    pub env: BTreeMap<String, String>,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            sanitize_vulkan: true,
            graphics: "dxvk".into(),
            // Default stays "noisy" so upgrades and first-time installs keep
            // failure output visible; quiet is an explicit player choice.
            log_level: "normal".into(),
            gamescope: GamescopeConfig::default(),
            env: builtin_launch_env(),
        }
    }
}

/// Wrap the game process in gamescope (nested compositor).
///
/// Kept as its own table so legacy configs without `[launch.gamescope]` stay
/// disabled and gain sensible fullscreen defaults when the user opts in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GamescopeConfig {
    /// Master switch. Default **false** so everyday launches stay plain umu/Proton.
    pub enabled: bool,
    /// Binary name or absolute path. `None` → `gamescope` on `PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Pass `-f` so gamescope itself is host-fullscreen (default true when used).
    pub fullscreen: bool,
    /// Pass `-b` (borderless nested window). Useful without host fullscreen.
    pub borderless: bool,
    /// Pass `--force-windows-fullscreen` so exclusive-FS games fill the nested
    /// display instead of a tiny letterboxed client. Default true.
    pub force_windows_fullscreen: bool,
    /// Optional host output width (`-W`). `None` → gamescope picks display size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Optional host output height (`-H`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Optional nested (game) width (`-w`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_width: Option<u32>,
    /// Optional nested (game) height (`-h`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_height: Option<u32>,
    /// Extra argv inserted before `--` (e.g. `["-S", "integer", "--adaptive-sync"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
}

impl Default for GamescopeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: None,
            // When the user turns gamescope on, they almost always want real FS.
            fullscreen: true,
            borderless: false,
            // Without this, many exclusive-FS titles stay a small window inside
            // gamescope; black “frames” / letterbox glitches are common on dual
            // monitors with bare Proton exclusive mode.
            force_windows_fullscreen: true,
            width: None,
            height: None,
            nested_width: None,
            nested_height: None,
            extra_args: Vec::new(),
        }
    }
}

impl GamescopeConfig {
    /// Argv for gamescope **before** the `--` separator (no binary name).
    pub fn build_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.fullscreen {
            args.push("-f".into());
        }
        if self.borderless {
            args.push("-b".into());
        }
        if self.force_windows_fullscreen {
            args.push("--force-windows-fullscreen".into());
        }
        if let Some(w) = self.width {
            args.push("-W".into());
            args.push(w.to_string());
        }
        if let Some(h) = self.height {
            args.push("-H".into());
            args.push(h.to_string());
        }
        if let Some(w) = self.nested_width {
            args.push("-w".into());
            args.push(w.to_string());
        }
        if let Some(h) = self.nested_height {
            args.push("-h".into());
            args.push(h.to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Short label for TUI / doctor (`off` or `on (-f …)`).
    pub fn status_label(&self) -> String {
        if !self.enabled {
            return "off".into();
        }
        let mut flags = Vec::new();
        if self.fullscreen {
            flags.push("-f");
        }
        if self.borderless {
            flags.push("-b");
        }
        if self.force_windows_fullscreen {
            flags.push("--force-windows-fullscreen");
        }
        if flags.is_empty() {
            "on".into()
        } else {
            format!("on ({})", flags.join(" "))
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
        // Missing [launch.env] gets built-in defaults (DXVK_HUD / STEAMOS / …).
        assert_eq!(legacy.launch.env.get("DXVK_HUD").map(String::as_str), Some("0"));
        assert_eq!(legacy.launch.env.get("STEAMOS").map(String::as_str), Some("1"));
        assert_eq!(
            legacy.launch.env.get("UMU_USE_STEAM").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn launch_env_roundtrip_and_explicit_empty() {
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

        let mut cfg = Config::default();
        cfg.launch.env.insert("FOO".into(), "bar".into());
        cfg.save(&paths).unwrap();
        let loaded = Config::load(&paths).unwrap();
        assert_eq!(loaded.launch.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(loaded.launch.env.get("DXVK_HUD").map(String::as_str), Some("0"));

        // Explicit empty [launch.env] must not resurrect built-ins.
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
[launch]
sanitize_vulkan = true
graphics = "dxvk"
log_level = "normal"
[launch.env]
"#,
        )
        .unwrap();
        let empty = Config::load(&paths).unwrap();
        assert!(empty.launch.env.is_empty());
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

    #[test]
    fn gamescope_defaults_and_args() {
        let g = GamescopeConfig::default();
        assert!(!g.enabled);
        assert!(g.fullscreen);
        assert!(g.force_windows_fullscreen);
        assert!(!g.borderless);
        // build_args still emit flags even when disabled — caller gates on enabled.
        let args = g.build_args();
        assert!(args.iter().any(|a| a == "-f"));
        assert!(args.iter().any(|a| a == "--force-windows-fullscreen"));
        assert!(!args.iter().any(|a| a == "-b"));

        let mut custom = GamescopeConfig::default();
        custom.enabled = true;
        custom.fullscreen = false;
        custom.borderless = true;
        custom.force_windows_fullscreen = false;
        custom.width = Some(2560);
        custom.height = Some(1440);
        custom.nested_width = Some(1280);
        custom.nested_height = Some(720);
        custom.extra_args = vec!["-S".into(), "integer".into()];
        assert_eq!(
            custom.build_args(),
            vec!["-b", "-W", "2560", "-H", "1440", "-w", "1280", "-h", "720", "-S", "integer"]
        );
        assert_eq!(GamescopeConfig::default().status_label(), "off");
        assert!(custom.status_label().starts_with("on"));
    }

    #[test]
    fn gamescope_legacy_config_loads_disabled() {
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
[launch]
sanitize_vulkan = true
graphics = "dxvk"
log_level = "normal"
"#,
        )
        .unwrap();
        let cfg = Config::load(&paths).unwrap();
        assert!(!cfg.launch.gamescope.enabled);
        assert!(cfg.launch.gamescope.fullscreen);
        assert!(cfg.launch.gamescope.force_windows_fullscreen);
    }
}
