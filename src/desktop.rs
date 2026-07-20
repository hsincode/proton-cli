//! Freedesktop .desktop shortcut generation.

use std::fs;
use std::process::Command;

use crate::apps::AppEntry;
use crate::config::DesktopConfig;
use crate::error::Result;
use crate::paths::Paths;

/// Write `~/.local/share/applications/proton-cli-<id>.desktop`.
pub fn install_shortcut(
    paths: &Paths,
    desktop_cfg: &DesktopConfig,
    app_id: &str,
    entry: &AppEntry,
) -> Result<std::path::PathBuf> {
    paths.ensure_dirs()?;
    let path = paths.desktop_file(app_id);
    let content = render_desktop(desktop_cfg, app_id, entry);
    fs::write(&path, content)?;
    // Best-effort: many DEs pick up new files without this.
    let _ = Command::new("update-desktop-database")
        .arg(&paths.applications_dir)
        .status();
    Ok(path)
}

pub fn uninstall_shortcut(paths: &Paths, app_id: &str) -> Result<bool> {
    let path = paths.desktop_file(app_id);
    if path.exists() {
        fs::remove_file(&path)?;
        let _ = Command::new("update-desktop-database")
            .arg(&paths.applications_dir)
            .status();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn render_desktop(desktop_cfg: &DesktopConfig, app_id: &str, entry: &AppEntry) -> String {
    let categories = if desktop_cfg.categories.is_empty() {
        "Game;".to_string()
    } else {
        let mut c = desktop_cfg.categories.join(";");
        if !c.ends_with(';') {
            c.push(';');
        }
        c
    };

    let icon = entry
        .icon
        .as_ref()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("application-x-executable");

    // Exec uses PATH lookup so cargo install / distro packages both work.
    // App ids are restricted to [a-z0-9-]+ so no shell quoting is required.
    let name = escape_desktop_string(&entry.name);
    format!(
        "\
[Desktop Entry]
Type=Application
Version=1.0
Name={name}
Comment=Launched via proton-cli
Exec=proton-cli launch {app_id}
TryExec=proton-cli
Icon={icon}
Terminal=false
Categories={categories}
StartupNotify=true
"
    )
}

/// Escape values for Desktop Entry string keys (not Exec).
fn escape_desktop_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn sample_entry() -> AppEntry {
        AppEntry {
            name: "Cool Game".into(),
            exe: PathBuf::from("/tmp/game.exe"),
            args: vec![],
            cwd: None,
            prefix: "cool-game".into(),
            proton: "dwproton".into(),
            game_id: "umu-default".into(),
            store: "none".into(),
            locale: None,
            graphics: None,
            env: BTreeMap::new(),
            icon: None,
            desktop: true,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn desktop_contains_launch() {
        let cfg = DesktopConfig::default();
        let text = render_desktop(&cfg, "cool-game", &sample_entry());
        assert!(text.contains("Exec=proton-cli launch cool-game"));
        assert!(text.contains("Name=Cool Game"));
        assert!(text.contains("Type=Application"));
        assert!(text.contains("Categories=Game;"));
    }
}
