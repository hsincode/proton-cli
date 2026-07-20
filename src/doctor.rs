//! Environment health checks for users and crates.io consumers.

use std::path::PathBuf;

use crate::config::Config;
use crate::launch;
use crate::paths::Paths;
use crate::proton;

pub struct DoctorReport {
    pub config_path: PathBuf,
    pub data_path: PathBuf,
    pub umu: UmuStatus,
    pub protons: Vec<proton::ProtonInstall>,
    pub default_proton: Option<PathBuf>,
    pub proton_cli_on_path: bool,
}

pub enum UmuStatus {
    Found(PathBuf),
    Missing,
}

pub fn run_doctor(paths: &Paths, config: &Config) -> DoctorReport {
    let umu = match launch::resolve_umu(&config.umu) {
        Ok(p) => UmuStatus::Found(p),
        Err(_) => UmuStatus::Missing,
    };

    let protons = proton::list_installs(paths).unwrap_or_default();
    let default_proton =
        proton::resolve_proton_path(paths, &config.proton, &config.proton.default).ok();

    let proton_cli_on_path = which::which("proton-cli").is_ok();

    DoctorReport {
        config_path: paths.config_dir.clone(),
        data_path: paths.data_dir.clone(),
        umu,
        protons,
        default_proton,
        proton_cli_on_path,
    }
}

pub fn print_report(report: &DoctorReport) {
    println!("proton-cli doctor");
    println!("-----------------");
    println!("root       : {}", report.data_path.display());
    println!("config     : {}", report.config_path.join("config.toml").display());

    match &report.umu {
        UmuStatus::Found(p) => println!("umu-run    : OK ({})", p.display()),
        UmuStatus::Missing => {
            println!("umu-run    : MISSING");
            println!("  Install umu-launcher, e.g.:");
            println!("    Arch:   pacman -S umu-launcher");
            println!("    Fedora: dnf install umu-launcher");
            println!("    Other:  https://github.com/Open-Wine-Components/umu-launcher");
        }
    }

    if report.protons.is_empty() {
        println!("Proton     : none found");
        println!("  Run: proton-cli proton install");
    } else {
        println!("Proton installs:");
        for p in &report.protons {
            println!("  [{}] {} ({})", p.source, p.name, p.path.display());
        }
    }

    match &report.default_proton {
        Some(p) => println!("default PROTONPATH: {}", p.display()),
        None => println!("default PROTONPATH: unresolved (install dwproton)"),
    }

    if report.proton_cli_on_path {
        println!("PATH       : proton-cli found (desktop shortcuts will work)");
    } else {
        println!("PATH       : proton-cli NOT on PATH");
        println!("  Desktop shortcuts use `Exec=proton-cli launch <id>`.");
        println!("  Ensure `~/.cargo/bin` (or install prefix) is on PATH.");
    }
}
