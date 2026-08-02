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
    /// Path to a leftover `proton-cli` binary (renamed to `protoncli` in 0.1.3).
    pub obsolete_proton_cli: Option<PathBuf>,
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

    let proton_cli_on_path = which::which("protoncli").is_ok();
    // Pre-0.1.3 binary name; if still present it never receives `make install` updates.
    let obsolete_proton_cli = which::which("proton-cli").ok();

    DoctorReport {
        config_path: paths.config_dir.clone(),
        data_path: paths.data_dir.clone(),
        umu,
        steam,
        backend,
        protons,
        default_proton,
        proton_cli_on_path,
        obsolete_proton_cli,
    }
}

pub fn print_report(report: &DoctorReport) {
    // Version of *this* process so a stale PATH binary is easy to spot after upgrades.
    println!("protoncli doctor v{}", env!("CARGO_PKG_VERSION"));
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
        println!("  Steam に Proton を入れるか: protoncli proton install");
    } else {
        println!("Proton installs (★ = default):");
        for p in &report.protons {
            let mark = report
                .default_proton
                .as_ref()
                .is_some_and(|d| d == &p.path);
            println!(
                "  {}[{}] {} ({})",
                if mark { "★ " } else { "  " },
                p.source,
                p.name,
                p.path.display()
            );
        }
    }

    match &report.default_proton {
        Some(p) => println!("default PROTONPATH: {}", p.display()),
        None => println!("default PROTONPATH: unresolved"),
    }

    if report.proton_cli_on_path {
        // Resolve again so users see the exact path desktop shortcuts will exec.
        match which::which("protoncli") {
            Ok(p) => println!("PATH       : protoncli → {}", p.display()),
            Err(_) => println!("PATH       : protoncli found (desktop shortcuts will work)"),
        }
    } else {
        println!("PATH       : protoncli NOT on PATH");
        println!("  Desktop shortcuts use `Exec=protoncli launch <id>`.");
        println!("  Ensure `~/.cargo/bin` (or install prefix) is on PATH.");
    }

    if let Some(old) = &report.obsolete_proton_cli {
        println!("obsolete   : proton-cli still present ({})", old.display());
        println!("  0.1.3 でコマンド名が protoncli に変わりました。");
        println!("  `make install` は protoncli だけ更新するため、古い proton-cli は古いまま残ります。");
        println!(
            "  削除: rm -f {}  または make install（掃除込み）",
            old.display()
        );
    }

    println!();
    println!("note: Steam は umu-run を同梱しません。");
    println!("      Proton + SteamLinuxRuntime があれば protoncli は Steam 経路で起動します。");
}
