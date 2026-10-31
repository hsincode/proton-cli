//! Health checks shown on the Doctor tab (and counted in the top bar).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::apps::AppsDb;
use crate::config::Config;
use crate::doctor::{self, SteamStatus, UmuStatus};
use crate::launch::LaunchBackend;
use crate::locale;
use crate::paths::Paths;
use crate::proton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Err,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

impl Check {
    fn new(level: Level, title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

pub fn build(
    paths: &Paths,
    config: &Config,
    db: &AppsDb,
    resolved: &BTreeMap<String, Option<PathBuf>>,
) -> Vec<Check> {
    let report = doctor::run_doctor(paths, config);
    let mut out = Vec::new();

    let steam_ready = matches!(report.steam, SteamStatus::Ready { .. });
    out.push(match &report.backend {
        Some(LaunchBackend::Umu { umu_run }) => Check::new(
            Level::Ok,
            "Launch backend: umu-run",
            umu_run.display().to_string(),
        ),
        Some(LaunchBackend::Steam { use_container, .. }) => Check::new(
            Level::Ok,
            "Launch backend: Steam Linux Runtime",
            format!("container = {use_container}"),
        ),
        None => Check::new(
            Level::Err,
            "No launch backend",
            "Install Steam (with Steam Linux Runtime + Proton) or umu-launcher",
        ),
    });

    out.push(match &report.umu {
        UmuStatus::Found(p) => Check::new(Level::Ok, "umu-run found", p.display().to_string()),
        UmuStatus::Missing if steam_ready => Check::new(
            Level::Info,
            "umu-run not installed",
            "Optional: Steam's runtime is used instead",
        ),
        UmuStatus::Missing => Check::new(
            Level::Warn,
            "umu-run not installed",
            "Install umu-launcher, or Steam with Steam Linux Runtime",
        ),
    });

    out.push(match &report.steam {
        SteamStatus::Ready {
            client, runtime, ..
        } => Check::new(
            Level::Ok,
            format!("Steam: {runtime}"),
            client.display().to_string(),
        ),
        SteamStatus::ClientOnly { client } => Check::new(
            Level::Info,
            "Steam client without Linux Runtime",
            client.display().to_string(),
        ),
        SteamStatus::Missing => Check::new(Level::Info, "Steam not found", "Optional with umu-run"),
    });

    let default = &config.proton.default;
    out.push(match &report.default_proton {
        Some(p) => Check::new(
            Level::Ok,
            format!("Default Proton: {default}"),
            p.display().to_string(),
        ),
        None => Check::new(
            Level::Err,
            format!("Default Proton `{default}` not found"),
            "Install dwproton on the Proton tab (i), or pick an installed build",
        ),
    });

    out.push(match config.locale.as_deref().and_then(locale::find) {
        Some(l) => Check::new(Level::Ok, format!("Locale: {}", l.label), l.posix),
        None => Check::new(
            Level::Warn,
            "Locale not set",
            "Choose one on the Settings tab",
        ),
    });

    for (id, e) in &db.apps {
        if !e.exe.is_file() {
            out.push(Check::new(
                Level::Warn,
                format!("{}: exe not found", e.name),
                e.exe.display().to_string(),
            ));
        }
        if !paths.prefix_path(&e.prefix).is_dir() {
            out.push(Check::new(
                Level::Warn,
                format!("{}: prefix `{}` is missing", e.name, e.prefix),
                format!("A fresh bottle is created on the next launch of `{id}`"),
            ));
        }
        if let Some(p) = e.proton.as_deref().filter(|p| !p.is_empty()) {
            let req = proton::effective_requested(Some(p), default);
            if resolved.get(req).is_some_and(Option::is_none) {
                out.push(Check::new(
                    Level::Err,
                    format!("{}: Proton `{p}` not found", e.name),
                    "Pick another build in Library → Proton",
                ));
            }
        }
    }

    if let Some(old) = &report.obsolete_proton_cli {
        out.push(Check::new(
            Level::Warn,
            "Old `proton-cli` binary on PATH",
            format!(
                "{} (renamed to protoncli in 0.1.3; remove it)",
                old.display()
            ),
        ));
    }
    if !report.proton_cli_on_path {
        out.push(Check::new(
            Level::Info,
            "protoncli is not on PATH",
            "Desktop shortcuts call `protoncli`; add ~/.cargo/bin to PATH",
        ));
    }

    // Problems first, keeping their discovery order.
    out.sort_by_key(|c| match c.level {
        Level::Err => 0,
        Level::Warn => 1,
        Level::Ok => 2,
        Level::Info => 3,
    });
    out
}
