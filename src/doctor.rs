//! Environment health checks for users and crates.io consumers.

use std::path::PathBuf;

use crate::config::Config;
use crate::launch::{self, LaunchBackend};
use crate::paths::Paths;
use crate::proton;
use crate::steam;

pub struct DoctorReport {
    pub config_path: PathBuf,
    pub data_path: PathBuf,
    pub umu: UmuStatus,
    pub steam: SteamStatus,
    pub backend: Option<LaunchBackend>,
    pub protons: Vec<proton::ProtonInstall>,
    pub default_proton: Option<PathBuf>,
    pub proton_cli_on_path: bool,
}

pub enum UmuStatus {
    Found(PathBuf),
    Missing,
}

pub enum SteamStatus {
    Ready {
        client: PathBuf,
        runtime: String,
        container: bool,
    },
    ClientOnly {
        client: PathBuf,
    },
    Missing,
}

pub fn run_doctor(paths: &Paths, config: &Config) -> DoctorReport {
    let umu = match launch::resolve_umu(&config.umu) {
        Ok(p) => UmuStatus::Found(p),
        Err(_) => UmuStatus::Missing,
    };

    let steam = match steam::resolve_steam_layout() {
        Some(layout) if steam::has_container_runtime(&layout) => SteamStatus::Ready {
            client: layout.client_install.clone(),
            runtime: layout.runtime_name.clone(),
            container: true,
        },
        Some(layout) => SteamStatus::ClientOnly {
            client: layout.client_install,
        },
        None => SteamStatus::Missing,
    };

    let backend = launch::resolve_backend(config).ok();

    let protons = proton::list_installs(paths).unwrap_or_default();
    let default_proton =
        proton::resolve_proton_path(paths, &config.proton, &config.proton.default).ok();

    let proton_cli_on_path = which::which("proton-cli").is_ok();

    DoctorReport {
        config_path: paths.config_dir.clone(),
        data_path: paths.data_dir.clone(),
        umu,
        steam,
        backend,
        protons,
        default_proton,
        proton_cli_on_path,
    }
}

pub fn print_report(report: &DoctorReport) {
    println!("proton-cli doctor");
    println!("-----------------");
    println!("root       : {}", report.data_path.display());
    println!(
        "config     : {}",
        report.config_path.join("config.toml").display()
    );

    match &report.umu {
        UmuStatus::Found(p) => println!("umu-run    : OK ({})", p.display()),
        UmuStatus::Missing => {
            println!("umu-run    : not found (optional if Steam is ready)");
            println!("  Steam があれば umu なしでも起動できます。");
            println!("  任意: pacman -S umu-launcher / dnf install umu-launcher");
            println!("  または Lutris の umu-run を自動検出します。");
        }
    }

    match &report.steam {
        SteamStatus::Ready {
            client,
            runtime,
            container,
        } => {
            println!(
                "Steam      : OK (client={}, runtime={}, container={})",
                client.display(),
                runtime,
                container
            );
        }
        SteamStatus::ClientOnly { client } => {
            println!(
                "Steam      : client OK ({}) but SteamLinuxRuntime missing",
                client.display()
            );
            println!(
                "  Steam を起動し、いずれかの Proton ゲームを一度走らせるか、"
            );
            println!("  ツール → ランタイム で Steam Linux Runtime を入れてください。");
        }
        SteamStatus::Missing => {
            println!("Steam      : not found");
            println!("  Steam か umu-launcher のどちらかが必要です。");
        }
    }

    match &report.backend {
        Some(LaunchBackend::Umu { umu_run }) => {
            println!("backend    : umu ({})", umu_run.display());
        }
        Some(LaunchBackend::Steam {
            layout,
            use_container,
        }) => {
            println!(
                "backend    : steam (runtime={}, container={})",
                if layout.runtime_name.is_empty() {
                    "none"
                } else {
                    &layout.runtime_name
                },
                use_container
            );
        }
        None => println!("backend    : NONE — cannot launch games yet"),
    }

    if report.protons.is_empty() {
        println!("Proton     : none found");
        println!("  Steam に Proton を入れるか: proton-cli proton install");
    } else {
        println!("Proton installs:");
        for p in &report.protons {
            println!("  [{}] {} ({})", p.source, p.name, p.path.display());
        }
    }

    match &report.default_proton {
        Some(p) => println!("default PROTONPATH: {}", p.display()),
        None => println!("default PROTONPATH: unresolved"),
    }

    if report.proton_cli_on_path {
        println!("PATH       : proton-cli found (desktop shortcuts will work)");
    } else {
        println!("PATH       : proton-cli NOT on PATH");
        println!("  Desktop shortcuts use `Exec=proton-cli launch <id>`.");
        println!("  Ensure `~/.cargo/bin` (or install prefix) is on PATH.");
    }

    println!();
    println!("note: Steam は umu-run を同梱しません。");
    println!("      Proton + SteamLinuxRuntime があれば proton-cli は Steam 経路で起動します。");
}
