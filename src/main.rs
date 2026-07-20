//! proton-cli — CLI launcher for Windows games via umu + Proton.

mod apps;
mod cli;
mod config;
mod desktop;
mod doctor;
mod error;
mod launch;
mod paths;
mod proton;
mod tui;

use std::process::ExitCode;

use clap::Parser;

use crate::apps::{new_entry, AppsDb};
use crate::cli::{Cli, Commands, DesktopCmd, PrefixCmd, ProtonCmd};
use crate::config::Config;
use crate::error::Error;
use crate::paths::{is_valid_app_id, slugify, Paths, DEFAULT_PREFIX_ID};

fn main() -> ExitCode {
    match try_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<ExitCode, Error> {
    let cli = Cli::parse();
    let paths = Paths::resolve()?;
    paths.ensure_dirs()?;
    let config = Config::load(&paths)?;

    let command = match cli.command {
        None | Some(Commands::Tui) => {
            tui::run(paths, config)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(cmd) => cmd,
    };

    match command {
        Commands::Tui => unreachable!("handled above"),

        Commands::Add {
            name,
            exe,
            id,
            args,
            cwd,
            prefix,
            proton,
            game_id,
            store,
            no_desktop,
        } => {
            let id = id.unwrap_or_else(|| slugify(&name));
            if !is_valid_app_id(&id) {
                return Err(Error::InvalidAppId(id));
            }
            // Default wineprefix is ~/.protoncli/default (id "default"), not per-app.
            let prefix = prefix.unwrap_or_else(|| config.default_prefix.clone());
            if !std::path::Path::new(&prefix).is_absolute()
                && prefix != DEFAULT_PREFIX_ID
                && !is_valid_app_id(&prefix)
            {
                return Err(Error::InvalidAppId(prefix));
            }
            let create_desktop = !no_desktop && config.desktop.create_on_add;
            let entry = new_entry(
                name,
                exe,
                args,
                cwd,
                prefix.clone(),
                proton,
                game_id,
                store,
                create_desktop,
            )?;

            let mut db = AppsDb::load(&paths)?;
            db.insert(id.clone(), entry.clone())?;
            db.save(&paths)?;

            launch::ensure_prefix(&paths.prefix_path(&prefix))?;

            if create_desktop {
                let desk = desktop::install_shortcut(&paths, &config.desktop, &id, &entry)?;
                println!("registered `{id}`");
                println!("  prefix : {}", paths.prefix_path(&prefix).display());
                println!("  desktop: {}", desk.display());
            } else {
                println!("registered `{id}` (no desktop shortcut)");
                println!("  prefix: {}", paths.prefix_path(&prefix).display());
            }
            Ok(ExitCode::SUCCESS)
        }

        Commands::Launch { id } => {
            let db = AppsDb::load(&paths)?;
            let entry = db.get(&id)?;
            let spec = launch::LaunchSpec::from_app(&paths, &config, entry)?;
            let status = launch::run_umu(&config, &spec)?;
            Ok(status_to_code(status))
        }

        Commands::Run {
            exe,
            args,
            prefix,
            proton,
            game_id,
            cwd,
        } => {
            let prefix_id = prefix.unwrap_or_else(|| config.default_prefix.clone());
            let prefix_path = paths.prefix_path(&prefix_id);
            let proton_path = proton::resolve_proton_path(&paths, &config.proton, &proton)?;
            let exe = if exe.is_absolute() {
                exe
            } else {
                std::env::current_dir()?.join(exe)
            };
            let cwd = cwd.or_else(|| exe.parent().map(|p| p.to_path_buf()));
            let spec = launch::LaunchSpec {
                exe,
                args,
                cwd,
                prefix: prefix_path,
                proton_path,
                game_id,
                store: "none".into(),
                env: vec![],
            };
            let status = launch::run_umu(&config, &spec)?;
            Ok(status_to_code(status))
        }

        Commands::List => {
            let db = AppsDb::load(&paths)?;
            if db.apps.is_empty() {
                println!("(no apps registered)");
            } else {
                println!("{:<20} {:<20} {:<12} {}", "ID", "NAME", "PREFIX", "EXE");
                for (id, e) in &db.apps {
                    println!(
                        "{:<20} {:<20} {:<12} {}",
                        id,
                        e.name,
                        e.prefix,
                        e.exe.display()
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Commands::Remove {
            id,
            keep_prefix,
            keep_desktop,
        } => {
            let mut db = AppsDb::load(&paths)?;
            let entry = db.remove(&id)?;
            db.save(&paths)?;

            if !keep_desktop && entry.desktop {
                desktop::uninstall_shortcut(&paths, &id)?;
            }
            if !keep_prefix {
                let p = paths.prefix_path(&entry.prefix);
                // Never auto-delete the shared default prefix unless it is only this app's.
                if entry.prefix == DEFAULT_PREFIX_ID {
                    println!(
                        "kept shared default prefix {} (use rm -rf manually if needed)",
                        p.display()
                    );
                } else if p.exists() {
                    std::fs::remove_dir_all(&p)?;
                    println!("removed prefix {}", p.display());
                }
            }
            println!("removed app `{id}`");
            Ok(ExitCode::SUCCESS)
        }

        Commands::Desktop { command } => match command {
            DesktopCmd::Install { id } => {
                let mut db = AppsDb::load(&paths)?;
                let entry = db.get(&id)?.clone();
                let path = desktop::install_shortcut(&paths, &config.desktop, &id, &entry)?;
                if let Some(e) = db.apps.get_mut(&id) {
                    e.desktop = true;
                }
                db.save(&paths)?;
                println!("desktop shortcut: {}", path.display());
                Ok(ExitCode::SUCCESS)
            }
            DesktopCmd::Uninstall { id } => {
                let removed = desktop::uninstall_shortcut(&paths, &id)?;
                let mut db = AppsDb::load(&paths)?;
                if let Some(e) = db.apps.get_mut(&id) {
                    e.desktop = false;
                }
                db.save(&paths)?;
                if removed {
                    println!("removed desktop shortcut for `{id}`");
                } else {
                    println!("no desktop shortcut for `{id}`");
                }
                Ok(ExitCode::SUCCESS)
            }
        },

        Commands::Proton { command } => match command {
            ProtonCmd::Install { version, force } => {
                let path = proton::install_dwproton(&paths, version.as_deref(), force)?;
                println!("PROTONPATH={}", path.display());
                Ok(ExitCode::SUCCESS)
            }
            ProtonCmd::List => {
                let installs = proton::list_installs(&paths)?;
                if installs.is_empty() {
                    println!("(no Proton installs found)");
                    println!("hint: proton-cli proton install");
                } else {
                    for p in installs {
                        println!("[{}] {}  {}", p.source, p.name, p.path.display());
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
        },

        Commands::Prefix { command } => match command {
            PrefixCmd::List => {
                for id in paths.list_prefix_ids()? {
                    let path = paths.prefix_path(&id);
                    let tag = if id == config.default_prefix {
                        " (default)"
                    } else {
                        ""
                    };
                    println!("{id}{tag}  {}", path.display());
                }
                Ok(ExitCode::SUCCESS)
            }
            PrefixCmd::Create { id } => {
                if !std::path::Path::new(&id).is_absolute()
                    && id != DEFAULT_PREFIX_ID
                    && !is_valid_app_id(&id)
                {
                    return Err(Error::InvalidAppId(id));
                }
                let path = paths.prefix_path(&id);
                launch::ensure_prefix(&path)?;
                println!("created {}", path.display());
                Ok(ExitCode::SUCCESS)
            }
            PrefixCmd::Path { id } => {
                println!("{}", paths.prefix_path(&id).display());
                Ok(ExitCode::SUCCESS)
            }
        },

        Commands::Doctor => {
            let report = doctor::run_doctor(&paths, &config);
            doctor::print_report(&report);
            println!(
                "default WINEPREFIX: {} (id=`{}`)",
                paths.prefix_path(&config.default_prefix).display(),
                config.default_prefix
            );
            let ok = matches!(report.umu, doctor::UmuStatus::Found(_))
                && report.default_proton.is_some();
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }

        Commands::Path { id } => {
            let db = AppsDb::load(&paths)?;
            let entry = db.get(&id)?;
            println!("id     : {id}");
            println!("name   : {}", entry.name);
            println!("exe    : {}", entry.exe.display());
            println!("prefix : {}", paths.prefix_path(&entry.prefix).display());
            println!("desktop: {}", paths.desktop_file(&id).display());
            match proton::resolve_proton_path(&paths, &config.proton, &entry.proton) {
                Ok(p) => println!("proton : {}", p.display()),
                Err(_) => println!("proton : (unresolved: {})", entry.proton),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn status_to_code(status: std::process::ExitStatus) -> ExitCode {
    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(status.code().unwrap_or(1) as u8)
    }
}
