//! Launch Windows executables via umu-run **or** Steam's own Proton + Linux Runtime.
//!
//! Preference order:
//! 1. `umu-run` (PATH, config, Lutris/Heroic bundles, managed install)
//! 2. Steam Linux Runtime + Proton (no separate umu package needed if Steam is installed)

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use which::which;

use crate::apps::AppEntry;
use crate::config::{self, Config, GamescopeConfig, UmuConfig};
use crate::error::{Error, Result};
use crate::locale;
use crate::paths::Paths;
use crate::proton;
use crate::steam::{self, SteamLayout};

/// Host env keys that pin Vulkan ICDs. Session configs sometimes set only the
/// x86_64 ICD (`VK_ICD_FILENAMES=…/radeon_icd.x86_64.json`), which makes
/// **32-bit** DXVK fail with `DxvkInstance::createInstance`. Steam and Lutris
/// do not inherit that pin into the game process; we strip them by default.
pub const HOST_VULKAN_ICD_KEYS: &[&str] = &["VK_ICD_FILENAMES", "VK_DRIVER_FILES"];

/// How a game will be started.
#[derive(Debug, Clone)]
pub enum LaunchBackend {
    /// Open-Wine-Components umu-run (+ its env: WINEPREFIX, PROTONPATH, …).
    Umu { umu_run: PathBuf },
    /// Valve Steam Linux Runtime container + Proton script (Steam install required).
    Steam {
        layout: SteamLayout,
        /// When true, wrap with `_v2-entry-point` (preferred).
        use_container: bool,
    },
}

/// Resolve umu-run from config, PATH, and common bundle locations.
pub fn resolve_umu(cfg: &UmuConfig) -> Result<PathBuf> {
    if let Some(cmd) = &cfg.command {
        let p = PathBuf::from(cmd);
        if p.is_absolute() && p.is_file() {
            return Ok(p);
        }
        if let Ok(w) = which(cmd) {
            return Ok(w);
        }
        return Err(Error::UmuNotFound);
    }

    if let Ok(w) = which("umu-run") {
        return Ok(w);
    }

    for candidate in umu_search_paths() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(Error::UmuNotFound)
}

fn umu_search_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(base) = directories::BaseDirs::new() {
        let home = base.home_dir();
        out.extend([
            // protoncli managed
            home.join(".protoncli/bin/umu-run"),
            home.join(".local/bin/umu-run"),
            // Lutris ships a zipapp here when Proton-GE is used
            home.join(".local/share/lutris/runtime/umu/umu-run"),
            // Heroic (various layouts)
            home.join(".config/heroic/tools/umu/umu-run"),
            home.join(".local/share/heroic/tools/umu/umu-run"),
            // Flatpak Lutris
            home.join(".var/app/net.lutris.Lutris/data/lutris/runtime/umu/umu-run"),
        ]);
    }
    out.extend([
        PathBuf::from("/usr/bin/umu-run"),
        PathBuf::from("/usr/local/bin/umu-run"),
    ]);
    out
}

/// Pick the best available backend: umu if present, else Steam+Proton runtime.
pub fn resolve_backend(config: &Config) -> Result<LaunchBackend> {
    // Explicit prefer via config.umu.prefer later; for now auto.
    match resolve_umu(&config.umu) {
        Ok(umu_run) => Ok(LaunchBackend::Umu { umu_run }),
        Err(_) => {
            let layout = steam::resolve_steam_layout().ok_or(Error::NoLaunchBackend)?;
            let use_container = steam::has_container_runtime(&layout);
            Ok(LaunchBackend::Steam {
                layout,
                use_container,
            })
        }
    }
}

pub struct LaunchSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Steam compatdata root (`STEAM_COMPAT_DATA_PATH`). Wine lives in `pfx/` under this.
    pub prefix: PathBuf,
    pub proton_path: PathBuf,
    pub game_id: String,
    pub store: String,
    /// When true, strip host Vulkan ICD pins before applying [`Self::env`].
    pub sanitize_vulkan: bool,
    /// Effective graphics backend id (`dxvk` / `wined3d`) for logs / doctor.
    pub graphics: String,
    /// Effective terminal log level (`quiet` / `normal` / `verbose`).
    pub log_level: String,
    /// Effective gamescope wrap (global defaults + optional per-app `enabled` override).
    pub gamescope: GamescopeConfig,
    pub env: Vec<(String, String)>,
}

impl LaunchSpec {
    pub fn from_app(paths: &Paths, config: &Config, entry: &AppEntry) -> Result<Self> {
        let prefix = paths.prefix_path(&entry.prefix);
        let proton_path = proton::resolve_proton_path(paths, &config.proton, &entry.proton)?;
        let graphics = effective_graphics(entry.graphics.as_deref(), &config.launch.graphics);
        let log_level = config::normalize_log_level(&config.launch.log_level);
        Ok(Self {
            exe: entry.exe.clone(),
            args: entry.args.clone(),
            cwd: entry.cwd.clone(),
            prefix,
            proton_path,
            game_id: entry.game_id.clone(),
            store: entry.store.clone(),
            sanitize_vulkan: config.launch.sanitize_vulkan,
            graphics: graphics.to_string(),
            log_level: log_level.to_string(),
            gamescope: effective_gamescope(entry.gamescope, &config.launch.gamescope),
            // Locale → graphics → global [launch.env] → (optional) per-app env.
            env: merge_launch_env(
                config,
                entry.locale.as_deref(),
                entry.graphics.as_deref(),
                app_env_extra(entry),
            ),
        })
    }
}

/// Per-app custom env only when `use_custom_env`; otherwise empty (global defaults alone).
pub fn app_env_extra(entry: &AppEntry) -> Vec<(String, String)> {
    if entry.use_custom_env {
        entry
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    } else {
        Vec::new()
    }
}

/// Resolve graphics: per-app override, else global `config.launch.graphics`.
pub fn effective_graphics<'a>(app: Option<&'a str>, global: &'a str) -> &'a str {
    let raw = app.unwrap_or(global);
    config::normalize_graphics(raw)
}

/// Resolve gamescope: clone global flags, then apply per-app `enabled` if set.
///
/// Width / fullscreen / extra_args always come from global `[launch.gamescope]`;
/// only the master switch is overridable per app (so one broken exclusive-FS
/// title can opt in without wrapping every app).
pub fn effective_gamescope(app: Option<bool>, global: &GamescopeConfig) -> GamescopeConfig {
    let mut g = global.clone();
    if let Some(enabled) = app {
        g.enabled = enabled;
    }
    g
}

/// Resolve `gamescope` binary from config or PATH.
pub fn resolve_gamescope(cfg: &GamescopeConfig) -> Result<PathBuf> {
    if let Some(cmd) = &cfg.command {
        let p = PathBuf::from(cmd);
        if p.is_absolute() && p.is_file() {
            return Ok(p);
        }
        if let Ok(w) = which(cmd) {
            return Ok(w);
        }
        return Err(Error::GamescopeNotFound);
    }
    which("gamescope").map_err(|_| Error::GamescopeNotFound)
}

/// Build the process that actually starts: plain backend, or `gamescope -- backend…`.
///
/// Env / cwd / stdio are applied by the caller after this returns so both paths
/// share the same Steam/umu wiring.
fn build_launch_command(
    backend: &Path,
    backend_args: &[impl AsRef<std::ffi::OsStr>],
    gamescope: &GamescopeConfig,
) -> Result<Command> {
    if !gamescope.enabled {
        let mut cmd = Command::new(backend);
        cmd.args(backend_args);
        return Ok(cmd);
    }
    let gs = resolve_gamescope(gamescope)?;
    let mut cmd = Command::new(gs);
    cmd.args(gamescope.build_args());
    cmd.arg("--");
    cmd.arg(backend);
    cmd.args(backend_args);
    Ok(cmd)
}

fn gamescope_banner(gamescope: &GamescopeConfig) -> String {
    if !gamescope.enabled {
        return "gamescope=off".into();
    }
    format!("gamescope={}", gamescope.status_label())
}

/// Env pairs for a graphics backend (`PROTON_USE_WINED3D` only for wined3d).
pub fn graphics_env(mode: &str) -> Vec<(String, String)> {
    match config::normalize_graphics(mode) {
        "wined3d" => vec![("PROTON_USE_WINED3D".into(), "1".into())],
        _ => Vec::new(),
    }
}

/// Build launch env: locale → graphics → global `[launch.env]` → extra pairs.
///
/// Locale: `app_locale` if set, else `config.locale`.
/// Graphics: `app_graphics` if set, else `config.launch.graphics`.
/// Global defaults: `config.launch.env` (built-ins include `DXVK_HUD=0`, `STEAMOS=1`, …).
/// Extra env (per-app custom or CLI) always overrides the same keys
/// (raw `LANG` / `PROTON_USE_WINED3D` / `DXVK_HUD` still win).
pub fn merge_launch_env(
    config: &Config,
    app_locale: Option<&str>,
    app_graphics: Option<&str>,
    extra: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut map = std::collections::BTreeMap::new();
    let effective_loc = locale::effective_id(app_locale, config.locale.as_deref());
    for (k, v) in locale::env_for_id(effective_loc) {
        map.insert(k, v);
    }
    let gfx = effective_graphics(app_graphics, &config.launch.graphics);
    for (k, v) in graphics_env(gfx) {
        map.insert(k, v);
    }
    for (k, v) in &config.launch.env {
        map.insert(k.clone(), v.clone());
    }
    for (k, v) in extra {
        map.insert(k, v);
    }
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn merge_locale_then_app_override() {
        let mut cfg = Config::default();
        cfg.locale = Some("ja".into());
        let env = merge_launch_env(
            &cfg,
            None,
            None,
            [("LANG".into(), "C".into()), ("FOO".into(), "bar".into())],
        );
        let get = |k: &str| {
            env.iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("LANG"), Some("C")); // raw env wins
        assert_eq!(get("LC_ALL"), Some("ja_JP.UTF-8"));
        assert_eq!(get("FOO"), Some("bar"));
        // Built-in global defaults are present unless overridden.
        assert_eq!(get("DXVK_HUD"), Some("0"));
        assert_eq!(get("STEAMOS"), Some("1"));
        assert_eq!(get("UMU_USE_STEAM"), Some("1"));
    }

    #[test]
    fn custom_env_overrides_global_defaults() {
        let mut cfg = Config::default();
        cfg.launch.env.insert("DXVK_HUD".into(), "0".into());
        let env = merge_launch_env(
            &cfg,
            None,
            None,
            [("DXVK_HUD".into(), "compiler".into())],
        );
        let get = |k: &str| {
            env.iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("DXVK_HUD"), Some("compiler"));
        assert_eq!(get("STEAMOS"), Some("1"));
    }

    #[test]
    fn app_env_extra_respects_use_custom_flag() {
        use crate::apps::AppEntry;
        use chrono::Utc;
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let mut env = BTreeMap::new();
        env.insert("FOO".into(), "bar".into());
        let entry = AppEntry {
            name: "t".into(),
            exe: PathBuf::from("/tmp/x.exe"),
            args: vec![],
            cwd: None,
            prefix: "default".into(),
            proton: "dwproton".into(),
            game_id: "umu-default".into(),
            store: "none".into(),
            locale: None,
            graphics: None,
            gamescope: None,
            use_custom_env: false,
            env: env.clone(),
            icon: None,
            desktop: false,
            created_at: Utc::now(),
        };
        assert!(app_env_extra(&entry).is_empty());
        let mut custom = entry;
        custom.use_custom_env = true;
        assert_eq!(app_env_extra(&custom), vec![("FOO".into(), "bar".into())]);
    }

    #[test]
    fn app_locale_overrides_global() {
        let mut cfg = Config::default();
        cfg.locale = Some("ja".into());
        let env = merge_launch_env(&cfg, Some("en"), None, std::iter::empty());
        let get = |k: &str| {
            env.iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(get("LC_ALL"), Some("en_US.UTF-8"));
    }

    #[test]
    fn wined3d_sets_proton_flag() {
        let mut cfg = Config::default();
        cfg.launch.graphics = "dxvk".into();
        let env = merge_launch_env(&cfg, None, Some("wined3d"), std::iter::empty());
        assert!(env
            .iter()
            .any(|(k, v)| k == "PROTON_USE_WINED3D" && v == "1"));
        let env_dxvk = merge_launch_env(&cfg, None, None, std::iter::empty());
        assert!(!env_dxvk.iter().any(|(k, _)| k == "PROTON_USE_WINED3D"));
    }

    #[test]
    fn host_vulkan_icd_keys_documented() {
        assert!(HOST_VULKAN_ICD_KEYS.contains(&"VK_ICD_FILENAMES"));
        assert!(HOST_VULKAN_ICD_KEYS.contains(&"VK_DRIVER_FILES"));
    }

    #[test]
    fn effective_gamescope_app_override() {
        let mut global = GamescopeConfig::default();
        global.enabled = false;
        global.fullscreen = true;
        assert!(!effective_gamescope(None, &global).enabled);
        assert!(effective_gamescope(Some(true), &global).enabled);
        assert!(!effective_gamescope(Some(false), &global).enabled);
        // Other flags stay global.
        let on = effective_gamescope(Some(true), &global);
        assert!(on.fullscreen);
        assert!(on.force_windows_fullscreen);
    }
}

/// Ensure compatdata / prefix parent exists (Proton creates `pfx/` on first run).
pub fn ensure_prefix(prefix: &Path) -> Result<()> {
    std::fs::create_dir_all(prefix)?;
    Ok(())
}

/// Spawn the game and wait; returns the process exit status.
pub fn run_game(config: &Config, spec: &LaunchSpec) -> Result<ExitStatus> {
    let backend = resolve_backend(config)?;
    ensure_prefix(&spec.prefix)?;

    if !spec.exe.exists() {
        return Err(Error::ExeNotFound(spec.exe.clone()));
    }
    if !proton::is_valid_proton_dir(&spec.proton_path) {
        return Err(Error::InvalidProtonPath(spec.proton_path.clone()));
    }

    match backend {
        LaunchBackend::Umu { umu_run } => run_via_umu(&umu_run, spec),
        LaunchBackend::Steam {
            layout,
            use_container,
        } => run_via_steam(&layout, use_container, spec),
    }
}

fn apply_common_env(cmd: &mut Command, spec: &LaunchSpec) {
    // Always clear host PROTON_USE_WINED3D so `spec.env` (from graphics mode)
    // is authoritative; re-set below if graphics/env asks for wined3d.
    cmd.env_remove("PROTON_USE_WINED3D");

    // Steam/Lutris-like: drop host ICD pins that break multi-arch DXVK.
    if spec.sanitize_vulkan {
        for key in HOST_VULKAN_ICD_KEYS {
            cmd.env_remove(*key);
        }
    }

    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
}

/// Attach stdio for the child according to `spec.log_level`.
///
/// `quiet` discards umu/Proton/Wine spam so daily launches stay readable;
/// switch back to `normal` when diagnosing a failed start.
fn apply_log_stdio(cmd: &mut Command, log_level: &str) {
    if config::normalize_log_level(log_level) == "quiet" {
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
    }
    // normal / verbose: inherit (default for Command)
}

fn log_banner(level: &str, message: &str) {
    match config::normalize_log_level(level) {
        "quiet" => {}
        _ => eprintln!("{message}"),
    }
}

fn run_via_umu(umu_run: &Path, spec: &LaunchSpec) -> Result<ExitStatus> {
    // umu accepts WINEPREFIX; modern umu maps it onto STEAM_COMPAT_DATA_PATH.
    // We pass both for maximum compatibility: data path = our prefix root, wine = pfx.
    let pfx = spec.prefix.join("pfx");
    let mut backend_args: Vec<std::ffi::OsString> = Vec::new();
    backend_args.push(spec.exe.as_os_str().to_owned());
    for a in &spec.args {
        backend_args.push(a.into());
    }
    let mut cmd = build_launch_command(umu_run, &backend_args, &spec.gamescope)?;
    cmd.env("WINEPREFIX", &pfx);
    cmd.env("STEAM_COMPAT_DATA_PATH", &spec.prefix);
    cmd.env("PROTONPATH", &spec.proton_path);
    cmd.env("GAMEID", &spec.game_id);
    cmd.env("STORE", &spec.store);
    apply_common_env(&mut cmd, spec);
    apply_log_stdio(&mut cmd, &spec.log_level);

    log_banner(
        &spec.log_level,
        &format!(
            "protoncli: backend=umu ({}) STEAM_COMPAT_DATA_PATH={} PROTONPATH={} graphics={} sanitize_vulkan={} log_level={} {}",
            umu_run.display(),
            spec.prefix.display(),
            spec.proton_path.display(),
            spec.graphics,
            spec.sanitize_vulkan,
            config::normalize_log_level(&spec.log_level),
            gamescope_banner(&spec.gamescope),
        ),
    );
    if config::normalize_log_level(&spec.log_level) == "verbose" {
        eprintln!(
            "protoncli: verbose: GAMEID={} STORE={} exe={} args={:?}",
            spec.game_id,
            spec.store,
            spec.exe.display(),
            spec.args
        );
        if spec.gamescope.enabled {
            eprintln!(
                "protoncli: verbose: gamescope args = {:?}",
                spec.gamescope.build_args()
            );
        }
        if !spec.env.is_empty() {
            eprintln!("protoncli: verbose: launch env = {:?}", spec.env);
        }
    }

    Ok(cmd.status()?)
}

fn run_via_steam(
    layout: &SteamLayout,
    use_container: bool,
    spec: &LaunchSpec,
) -> Result<ExitStatus> {
    let proton_script = spec.proton_path.join("proton");
    if !proton_script.is_file() {
        return Err(Error::InvalidProtonPath(spec.proton_path.clone()));
    }

    // Required by Proton's python launcher (see CompatData).
    let base_env: Vec<(&str, PathBuf)> = vec![
        (
            "STEAM_COMPAT_CLIENT_INSTALL_PATH",
            layout.client_install.clone(),
        ),
        ("STEAM_COMPAT_DATA_PATH", spec.prefix.clone()),
    ];
    // Harmless defaults used by some Proton tooling
    let steam_app_id = extract_numeric_game_id(&spec.game_id).unwrap_or(0);

    if use_container {
        // Mirror Steam: runtime entry → proton waitforexitandrun → exe
        // (optionally wrapped: gamescope -- runtime …)
        let mut backend_args: Vec<std::ffi::OsString> = vec![
            "--verb=waitforexitandrun".into(),
            "--".into(),
            proton_script.as_os_str().to_owned(),
            "waitforexitandrun".into(),
            spec.exe.as_os_str().to_owned(),
        ];
        for a in &spec.args {
            backend_args.push(a.into());
        }
        let mut cmd = build_launch_command(&layout.runtime_entry, &backend_args, &spec.gamescope)?;
        for (k, v) in &base_env {
            cmd.env(k, v);
        }
        cmd.env("SteamAppId", steam_app_id.to_string());
        cmd.env("SteamGameId", steam_app_id.to_string());
        cmd.env("STEAM_COMPAT_APP_ID", steam_app_id.to_string());
        // Optional store/game id for protonfixes bundled in GE/dwproton
        cmd.env("GAMEID", &spec.game_id);
        cmd.env("STORE", &spec.store);
        apply_common_env(&mut cmd, spec);
        apply_log_stdio(&mut cmd, &spec.log_level);

        log_banner(
            &spec.log_level,
            &format!(
                "protoncli: backend=steam+{} client={} STEAM_COMPAT_DATA_PATH={} PROTON={} graphics={} sanitize_vulkan={} log_level={} {}",
                layout.runtime_name,
                layout.client_install.display(),
                spec.prefix.display(),
                spec.proton_path.display(),
                spec.graphics,
                spec.sanitize_vulkan,
                config::normalize_log_level(&spec.log_level),
                gamescope_banner(&spec.gamescope),
            ),
        );
        if config::normalize_log_level(&spec.log_level) == "verbose" {
            eprintln!(
                "protoncli: verbose: GAMEID={} STORE={} SteamAppId={} exe={} args={:?}",
                spec.game_id,
                spec.store,
                steam_app_id,
                spec.exe.display(),
                spec.args
            );
            if spec.gamescope.enabled {
                eprintln!(
                    "protoncli: verbose: gamescope args = {:?}",
                    spec.gamescope.build_args()
                );
            }
            if !spec.env.is_empty() {
                eprintln!("protoncli: verbose: launch env = {:?}", spec.env);
            }
        }

        return Ok(cmd.status()?);
    }

    // Bare proton (no pressure-vessel) — last resort when SLR is missing.
    let mut backend_args: Vec<std::ffi::OsString> = vec![
        "run".into(),
        spec.exe.as_os_str().to_owned(),
    ];
    for a in &spec.args {
        backend_args.push(a.into());
    }
    let mut cmd = build_launch_command(&proton_script, &backend_args, &spec.gamescope)?;
    for (k, v) in &base_env {
        cmd.env(k, v);
    }
    cmd.env("SteamAppId", steam_app_id.to_string());
    cmd.env("SteamGameId", steam_app_id.to_string());
    cmd.env("STEAM_COMPAT_APP_ID", steam_app_id.to_string());
    cmd.env("GAMEID", &spec.game_id);
    cmd.env("STORE", &spec.store);
    apply_common_env(&mut cmd, spec);
    apply_log_stdio(&mut cmd, &spec.log_level);

    log_banner(
        &spec.log_level,
        &format!(
            "protoncli: backend=steam-bare (no SLR container) client={} STEAM_COMPAT_DATA_PATH={} PROTON={} graphics={} sanitize_vulkan={} log_level={} {}",
            layout.client_install.display(),
            spec.prefix.display(),
            spec.proton_path.display(),
            spec.graphics,
            spec.sanitize_vulkan,
            config::normalize_log_level(&spec.log_level),
            gamescope_banner(&spec.gamescope),
        ),
    );
    // Setup problem (missing SLR), not game chatter — always surface once.
    eprintln!("protoncli: warning: install SteamLinuxRuntime (via Steam) for better compatibility");

    Ok(cmd.status()?)
}

/// Parse trailing digits from game ids like `umu-12345` or plain `348550`.
fn extract_numeric_game_id(game_id: &str) -> Option<u32> {
    let digits: String = game_id
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

// Keep old name as alias for call sites that still say run_umu
pub fn run_umu(config: &Config, spec: &LaunchSpec) -> Result<ExitStatus> {
    run_game(config, spec)
}
