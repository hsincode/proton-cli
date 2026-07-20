//! Launch Windows executables via umu-run + Proton.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use which::which;

use crate::apps::AppEntry;
use crate::config::{Config, UmuConfig};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::proton;

/// Resolve umu-run binary from config or PATH.
pub fn resolve_umu(cfg: &UmuConfig) -> Result<PathBuf> {
    if let Some(cmd) = &cfg.command {
        let p = PathBuf::from(cmd);
        if p.is_absolute() && p.is_file() {
            return Ok(p);
        }
        return which(cmd).map_err(|_| Error::UmuNotFound);
    }
    which("umu-run").map_err(|_| Error::UmuNotFound)
}

pub struct LaunchSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub prefix: PathBuf,
    pub proton_path: PathBuf,
    pub game_id: String,
    pub store: String,
    pub env: Vec<(String, String)>,
}

impl LaunchSpec {
    pub fn from_app(
        paths: &Paths,
        config: &Config,
        entry: &AppEntry,
    ) -> Result<Self> {
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
            env: entry.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        })
    }
}

/// Ensure prefix directory exists (Proton/umu initialize wineprefix on first run).
pub fn ensure_prefix(prefix: &Path) -> Result<()> {
    std::fs::create_dir_all(prefix)?;
    Ok(())
}

/// Spawn umu-run and wait; returns the process exit status.
pub fn run_umu(config: &Config, spec: &LaunchSpec) -> Result<ExitStatus> {
    let umu = resolve_umu(&config.umu)?;
    ensure_prefix(&spec.prefix)?;

    if !spec.exe.exists() {
        return Err(Error::ExeNotFound(spec.exe.clone()));
    }
    if !proton::is_valid_proton_dir(&spec.proton_path) {
        return Err(Error::InvalidProtonPath(spec.proton_path.clone()));
    }

    let mut cmd = Command::new(&umu);
    cmd.env("WINEPREFIX", &spec.prefix);
    cmd.env("PROTONPATH", &spec.proton_path);
    cmd.env("GAMEID", &spec.game_id);
    cmd.env("STORE", &spec.store);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }

    cmd.arg(&spec.exe);
    cmd.args(&spec.args);

    eprintln!(
        "proton-cli: umu-run WINEPREFIX={} PROTONPATH={}",
        spec.prefix.display(),
        spec.proton_path.display()
    );

    let status = cmd.status()?;
    Ok(status)
}
