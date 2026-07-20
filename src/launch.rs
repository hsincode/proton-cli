//! Launch Windows executables via umu-run **or** Steam's own Proton + Linux Runtime.
//!
//! Preference order:
//! 1. `umu-run` (PATH, config, Lutris/Heroic bundles, managed install)
//! 2. Steam Linux Runtime + Proton (no separate umu package needed if Steam is installed)

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use which::which;

use crate::apps::AppEntry;
use crate::config::{Config, UmuConfig};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::proton;
use crate::steam::{self, SteamLayout};

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
            // proton-cli managed
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
    pub env: Vec<(String, String)>,
}

impl LaunchSpec {
    pub fn from_app(paths: &Paths, config: &Config, entry: &AppEntry) -> Result<Self> {
        let prefix = paths.prefix_path(&entry.prefix);
        let proton_path = proton::resolve_proton_path(paths, &config.proton, &entry.proton)?;
        Ok(Self {
            exe: entry.exe.clone(),
            args: entry.args.clone(),
            cwd: entry.cwd.clone(),
            prefix,
            proton_path,
            game_id: entry.game_id.clone(),
            store: entry.store.clone(),
            env: entry
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
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
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
}

fn run_via_umu(umu_run: &Path, spec: &LaunchSpec) -> Result<ExitStatus> {
    // umu accepts WINEPREFIX; modern umu maps it onto STEAM_COMPAT_DATA_PATH.
    // We pass both for maximum compatibility: data path = our prefix root, wine = pfx.
    let pfx = spec.prefix.join("pfx");
    let mut cmd = Command::new(umu_run);
    cmd.env("WINEPREFIX", &pfx);
    cmd.env("STEAM_COMPAT_DATA_PATH", &spec.prefix);
    cmd.env("PROTONPATH", &spec.proton_path);
    cmd.env("GAMEID", &spec.game_id);
    cmd.env("STORE", &spec.store);
    apply_common_env(&mut cmd, spec);
    cmd.arg(&spec.exe);
    cmd.args(&spec.args);

    eprintln!(
        "proton-cli: backend=umu ({}) STEAM_COMPAT_DATA_PATH={} PROTONPATH={}",
        umu_run.display(),
        spec.prefix.display(),
        spec.proton_path.display()
    );

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
        let mut cmd = Command::new(&layout.runtime_entry);
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

        cmd.arg("--verb=waitforexitandrun");
        cmd.arg("--");
        cmd.arg(&proton_script);
        cmd.arg("waitforexitandrun");
        cmd.arg(&spec.exe);
        cmd.args(&spec.args);

        eprintln!(
            "proton-cli: backend=steam+{} client={} STEAM_COMPAT_DATA_PATH={} PROTON={}",
            layout.runtime_name,
            layout.client_install.display(),
            spec.prefix.display(),
            spec.proton_path.display()
        );

        return Ok(cmd.status()?);
    }

    // Bare proton (no pressure-vessel) — last resort when SLR is missing.
    let mut cmd = Command::new(&proton_script);
    for (k, v) in &base_env {
        cmd.env(k, v);
    }
    cmd.env("SteamAppId", steam_app_id.to_string());
    cmd.env("SteamGameId", steam_app_id.to_string());
    cmd.env("STEAM_COMPAT_APP_ID", steam_app_id.to_string());
    cmd.env("GAMEID", &spec.game_id);
    cmd.env("STORE", &spec.store);
    apply_common_env(&mut cmd, spec);
    cmd.arg("run");
    cmd.arg(&spec.exe);
    cmd.args(&spec.args);

    eprintln!(
        "proton-cli: backend=steam-bare (no SLR container) client={} STEAM_COMPAT_DATA_PATH={} PROTON={}",
        layout.client_install.display(),
        spec.prefix.display(),
        spec.proton_path.display()
    );
    eprintln!("proton-cli: warning: install SteamLinuxRuntime (via Steam) for better compatibility");

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
