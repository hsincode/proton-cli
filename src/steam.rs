//! Locate Steam install + Steam Linux Runtime for umu-less launches.
//!
//! Steam ships Proton and the container runtime (pressure-vessel / SLR), but **not**
//! `umu-run`. When umu is missing we launch like Steam does:
//!
//! ```text
//! SteamLinuxRuntime_*/_v2-entry-point --verb=waitforexitandrun -- \
//!   $PROTON/proton waitforexitandrun game.exe
//! ```
//!
//! with `STEAM_COMPAT_CLIENT_INSTALL_PATH` and `STEAM_COMPAT_DATA_PATH`.

use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

/// Resolved Steam layout used for native Proton launches.
#[derive(Debug, Clone)]
pub struct SteamLayout {
    /// Steam client root (contains `steam.sh`, `steamapps`, …).
    pub client_install: PathBuf,
    /// `_v2-entry-point` of a Steam Linux Runtime (sniper / soldier / …).
    pub runtime_entry: PathBuf,
    /// Human-readable runtime name for logs/doctor.
    pub runtime_name: String,
}

/// Candidate Steam roots (canonicalized when possible).
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |p: PathBuf| {
        let p = fs::canonicalize(&p).unwrap_or(p);
        if p.is_dir() && !roots.iter().any(|r| r == &p) {
            // Prefer directories that look like a Steam install.
            if p.join("steam.sh").is_file()
                || p.join("steamapps").is_dir()
                || p.join("steamapps/common").is_dir()
            {
                roots.push(p);
            }
        }
    };

    if let Ok(env) = std::env::var("STEAM_DIR") {
        push(PathBuf::from(env));
    }
    if let Ok(env) = std::env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        push(PathBuf::from(env));
    }

    if let Some(base) = BaseDirs::new() {
        let home = base.home_dir();
        for rel in [
            ".local/share/Steam",
            ".steam/steam",
            ".steam/root",
            ".steam/debian-installation",
            "Steam", // uncommon but seen
        ] {
            push(home.join(rel));
        }
    }

    // Flatpak Steam (optional)
    if let Some(base) = BaseDirs::new() {
        push(
            base.home_dir()
                .join(".var/app/com.valvesoftware.Steam/data/Steam"),
        );
    }

    roots
}

/// Preferred order of Steam Linux Runtime directory names under steamapps/common.
const RUNTIME_DIR_NAMES: &[&str] = &[
    "SteamLinuxRuntime_sniper", // Proton 8+
    "SteamLinuxRuntime_soldier",
    "SteamLinuxRuntime_medic",
    "SteamLinuxRuntime_4", // newer steamrt4 branding in some installs
    "SteamLinuxRuntime",
];

fn entry_point_in(dir: &Path) -> Option<PathBuf> {
    let ep = dir.join("_v2-entry-point");
    if ep.is_file() {
        Some(ep)
    } else {
        None
    }
}

/// Find a usable Steam layout (client root + runtime entry point).
pub fn resolve_steam_layout() -> Option<SteamLayout> {
    let roots = steam_roots();
    if roots.is_empty() {
        return None;
    }

    // 1) Official runtimes next to Steam library
    for root in &roots {
        let common = root.join("steamapps/common");
        if !common.is_dir() {
            continue;
        }
        for name in RUNTIME_DIR_NAMES {
            let dir = common.join(name);
            if let Some(ep) = entry_point_in(&dir) {
                return Some(SteamLayout {
                    client_install: root.clone(),
                    runtime_entry: ep,
                    runtime_name: (*name).to_string(),
                });
            }
        }
        // Any SteamLinuxRuntime_* with entry point
        if let Ok(rd) = fs::read_dir(&common) {
            let mut extras: Vec<(String, PathBuf)> = Vec::new();
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("SteamLinuxRuntime") {
                    if let Some(ep) = entry_point_in(&p) {
                        extras.push((name, ep));
                    }
                }
            }
            extras.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some((name, ep)) = extras.into_iter().next_back() {
                return Some(SteamLayout {
                    client_install: root.clone(),
                    runtime_entry: ep,
                    runtime_name: name,
                });
            }
        }
    }

    // 2) umu/Heroic/Lutris-cached steamrt under ~/.local/share/umu (no umu-run required)
    if let Some(base) = BaseDirs::new() {
        let umu_share = base.home_dir().join(".local/share/umu");
        for name in ["steamrt4", "steamrt3", "steamrt"] {
            let dir = umu_share.join(name);
            if let Some(ep) = entry_point_in(&dir) {
                let client = roots
                    .first()
                    .cloned()
                    .unwrap_or_else(|| base.home_dir().join(".local/share/Steam"));
                return Some(SteamLayout {
                    client_install: client,
                    runtime_entry: ep,
                    runtime_name: format!("umu-cache/{name}"),
                });
            }
        }
    }

    // Client found but no container runtime — still useful for bare `proton run`
    if let Some(root) = roots.into_iter().next() {
        return Some(SteamLayout {
            client_install: root,
            runtime_entry: PathBuf::new(),
            runtime_name: String::new(),
        });
    }

    None
}

pub fn has_container_runtime(layout: &SteamLayout) -> bool {
    !layout.runtime_entry.as_os_str().is_empty() && layout.runtime_entry.is_file()
}


