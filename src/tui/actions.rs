//! Mutations: every change is saved to disk, then cached state is reloaded.
//!
//! Functions returning `Result<std::result::Result<_, String>>` report
//! validation problems as `Err(String)` so the calling form can show them
//! inline; the outer `Result` is for I/O failures.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, TryRecvError};

use crate::apps::{self, new_entry, AppEntry, AppsDb, DeletePrefixOutcome};
use crate::browse;
use crate::config;
use crate::desktop;
use crate::error::{Error, Result};
use crate::launch;
use crate::locale;
use crate::paths::{is_valid_app_id, DEFAULT_PREFIX_ID};
use crate::proton::{self, InstallProgress};

use super::app::{App, Field, InstallTask, TaskMsg, ToastKind};
use super::overlay::{
    locale_options, proton_options, AddForm, EnvForm, EnvScope, GAMESCOPE_OPTIONS, GRAPHICS_OPTIONS,
};

type Check<T> = std::result::Result<T, String>;

fn step(cur: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (cur as i32 + dir).rem_euclid(len as i32) as usize
}

impl App {
    /// Load → mutate one entry → save, refreshing its .desktop file if asked.
    fn edit_app(
        &mut self,
        id: &str,
        refresh_desktop: bool,
        f: impl FnOnce(&mut AppEntry),
    ) -> Result<()> {
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))?;
        f(entry);
        let snapshot = entry.clone();
        db.save(&self.paths)?;
        if refresh_desktop && snapshot.desktop {
            if let Err(e) =
                desktop::install_shortcut(&self.paths, &self.config.desktop, id, &snapshot)
            {
                self.notify(
                    ToastKind::Warn,
                    format!("Desktop shortcut not updated: {e}"),
                );
            }
        }
        self.reload()
    }

    fn save_config(&mut self) -> Result<()> {
        self.config.save(&self.paths)?;
        self.reload()
    }

    fn app_name(&self, id: &str) -> String {
        self.db
            .apps
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    // --- library ------------------------------------------------------------

    pub fn launch(&mut self, id: &str) -> Result<()> {
        let name = self.app_name(id);
        let (paths, config) = (self.paths.clone(), self.config.clone());
        let res = super::suspend(|| {
            let db = AppsDb::load(&paths)?;
            let spec = launch::LaunchSpec::from_app(&paths, &config, db.get(id)?)?;
            launch::run_umu(&config, &spec)?;
            Ok(())
        });
        // The alternate screen was left either way; repaint everything.
        self.needs_full_redraw = true;
        match res {
            Ok(()) => self.notify(ToastKind::Info, format!("{name} exited")),
            Err(e) => self.notify(ToastKind::Err, format!("Launch failed: {e}")),
        }
        self.reload()
    }

    pub fn remove_app(&mut self, id: &str) -> Result<()> {
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db.remove(id)?;
        db.save(&self.paths)?;
        if entry.desktop {
            let _ = desktop::uninstall_shortcut(&self.paths, id);
        }
        self.notify(
            ToastKind::Ok,
            format!("Removed {} (prefix `{}` kept)", entry.name, entry.prefix),
        );
        self.reload()
    }

    /// ←/→ on a cycling detail row of the selected app.
    pub fn cycle_field(&mut self, dir: i32) -> Result<()> {
        let Some((id, entry)) = self.selected_app() else {
            return Ok(());
        };
        match self.current_field() {
            Field::Proton => {
                let opts = proton_options(&self.protons);
                let cur = opts.iter().position(|o| *o == entry.proton).unwrap_or(0);
                let next = opts[step(cur, opts.len(), dir)].clone();
                self.set_app_proton(&id, next)
            }
            Field::Locale => {
                let opts = locale_options();
                let cur = opts
                    .iter()
                    .position(|o| o.map(str::to_string) == entry.locale)
                    .unwrap_or(0);
                let next = opts[step(cur, opts.len(), dir)].map(str::to_string);
                self.set_app_locale(&id, next)
            }
            Field::Graphics => {
                let cur = match entry.graphics.as_deref() {
                    None => 0,
                    Some("wined3d") => 2,
                    Some(_) => 1,
                };
                let next = GRAPHICS_OPTIONS[step(cur, 3, dir)].map(str::to_string);
                self.set_app_graphics(&id, next)
            }
            Field::Gamescope => {
                let cur = GAMESCOPE_OPTIONS
                    .iter()
                    .position(|o| *o == entry.gamescope)
                    .unwrap_or(0);
                self.set_app_gamescope(&id, GAMESCOPE_OPTIONS[step(cur, 3, dir)])
            }
            _ => Ok(()),
        }
    }

    pub fn set_app_proton(&mut self, id: &str, value: Option<String>) -> Result<()> {
        self.edit_app(id, false, |e| e.proton = value)
    }

    pub fn set_app_locale(&mut self, id: &str, value: Option<String>) -> Result<()> {
        self.edit_app(id, false, |e| e.locale = value)
    }

    pub fn set_app_graphics(&mut self, id: &str, value: Option<String>) -> Result<()> {
        self.edit_app(id, false, |e| e.graphics = value)
    }

    pub fn set_app_gamescope(&mut self, id: &str, value: Option<bool>) -> Result<()> {
        self.edit_app(id, false, |e| e.gamescope = value)
    }

    pub fn set_app_name(&mut self, id: &str, name: &str) -> Result<Check<()>> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Ok(Err("Name cannot be empty".into()));
        }
        self.edit_app(id, true, |e| e.name = name.clone())?;
        self.select_app(id);
        self.notify(ToastKind::Ok, format!("Renamed to {name}"));
        Ok(Ok(()))
    }

    pub fn set_app_exe(&mut self, id: &str, exe: PathBuf) -> Result<Check<()>> {
        if !exe.is_file() {
            return Ok(Err(format!("Not found: {}", exe.display())));
        }
        if !browse::is_windows_exe(&exe) {
            return Ok(Err(format!("Not a .exe: {}", exe.display())));
        }
        self.edit_app(id, true, |e| {
            // Launch from the binary's directory, as at registration.
            e.cwd = exe.parent().map(Path::to_path_buf);
            e.exe = exe.clone();
        })?;
        self.notify(ToastKind::Ok, format!("Exe set to {}", exe.display()));
        Ok(Ok(()))
    }

    pub fn toggle_app_custom_env(&mut self, id: &str) -> Result<()> {
        self.edit_app(id, false, |e| e.use_custom_env = !e.use_custom_env)
    }

    pub fn delete_app_env(&mut self, id: &str, key: &str) -> Result<()> {
        self.edit_app(id, false, |e| {
            e.env.remove(key);
        })?;
        self.notify(ToastKind::Ok, format!("Removed {key}"));
        Ok(())
    }

    /// Validate and save an env form (global or per-app).
    pub fn save_env(&mut self, form: &EnvForm) -> Result<Check<String>> {
        let key = form.key.trimmed().to_string();
        if key.is_empty() {
            return Ok(Err("Key cannot be empty".into()));
        }
        if key.contains(['=', '\0', '\n']) || key.contains(char::is_whitespace) {
            return Ok(Err(
                "Key cannot contain '=', spaces or control characters".into()
            ));
        }
        let value = form.value.value().to_string();
        let original = form.original.clone();
        match &form.scope {
            EnvScope::Global => {
                if let Some(old) = original.filter(|o| o != &key) {
                    self.config.launch.env.remove(&old);
                }
                self.config.launch.env.insert(key.clone(), value);
                self.save_config()?;
            }
            EnvScope::App(id) => {
                self.edit_app(id, false, |e| {
                    if let Some(old) = original.filter(|o| o != &key) {
                        e.env.remove(&old);
                    }
                    e.env.insert(key.clone(), value);
                    // Extras only apply in custom mode.
                    e.use_custom_env = true;
                })?;
            }
        }
        self.notify(ToastKind::Ok, format!("Saved {key}"));
        Ok(Ok(key))
    }

    pub fn register(&mut self, form: &AddForm) -> Result<Check<String>> {
        if form.exe.is_empty() {
            return Ok(Err("Pick an exe first".into()));
        }
        let exe = PathBuf::from(form.exe.trimmed());
        let name = form.resolved_name();
        let id = form.resolved_id();
        if !is_valid_app_id(&id) {
            return Ok(Err(format!(
                "Invalid id `{id}`: use lowercase letters, digits and hyphens"
            )));
        }
        if self.db.apps.contains_key(&id) {
            return Ok(Err(format!("An app with id `{id}` already exists")));
        }
        let Some(prefix) = form.resolved_prefix(&self.config.default_prefix) else {
            return Ok(Err("Type a prefix id".into()));
        };
        if !Path::new(&prefix).is_absolute()
            && prefix != DEFAULT_PREFIX_ID
            && !is_valid_app_id(&prefix)
        {
            return Ok(Err(format!("Invalid prefix id `{prefix}`")));
        }
        let proton = proton_options(&self.protons)
            .get(form.proton_sel)
            .cloned()
            .flatten();
        let create_desktop = self.config.desktop.create_on_add;
        let entry = match new_entry(
            name.clone(),
            exe,
            vec![],
            None,
            prefix.clone(),
            proton,
            None,
            None,
            create_desktop,
        ) {
            Ok(e) => e,
            Err(e) => return Ok(Err(e.to_string())),
        };
        let mut db = AppsDb::load(&self.paths)?;
        if let Err(e) = db.insert(id.clone(), entry.clone()) {
            return Ok(Err(e.to_string()));
        }
        db.save(&self.paths)?;
        launch::ensure_prefix(&self.paths.prefix_path(&prefix))?;
        self.notify(ToastKind::Ok, format!("Added {name}"));
        if create_desktop {
            if let Err(e) =
                desktop::install_shortcut(&self.paths, &self.config.desktop, &id, &entry)
            {
                self.notify(
                    ToastKind::Warn,
                    format!("Added {name}, but the desktop shortcut failed: {e}"),
                );
            }
        }
        self.lib.filter.clear();
        self.reload()?;
        self.select_app(&id);
        Ok(Ok(id))
    }

    // --- proton ---------------------------------------------------------------

    pub fn set_default_proton(&mut self, idx: usize) -> Result<()> {
        let Some(inst) = self.protons.get(idx).cloned() else {
            return Ok(());
        };
        // resolve_proton_path matches by name; align prefer with the source so
        // same-named builds resolve to this tree.
        self.config.proton.default = inst.name.clone();
        self.config.proton.prefer = inst.source.to_string();
        self.config.proton.explicit_path = None;
        self.save_config()?;
        self.notify(ToastKind::Ok, format!("Default Proton: {}", inst.name));
        Ok(())
    }

    pub fn use_latest_dwproton(&mut self) -> Result<()> {
        self.config.proton.default = "dwproton".into();
        self.config.proton.explicit_path = None;
        self.save_config()?;
        if let Some(i) = self.active_proton_index() {
            self.proton_sel = i;
        }
        let desc = self.describe_proton("dwproton");
        self.notify(ToastKind::Ok, format!("Default Proton: {desc}"));
        Ok(())
    }

    pub fn toggle_prefer(&mut self) -> Result<()> {
        let next = if self.config.proton.prefer == "managed" {
            "steam"
        } else {
            "managed"
        };
        self.config.proton.prefer = next.into();
        self.save_config()?;
        self.notify(
            ToastKind::Ok,
            format!("Prefer {next} builds when resolving `dwproton`"),
        );
        Ok(())
    }

    pub fn start_install(&mut self) {
        if self.task.is_some() {
            self.notify(ToastKind::Info, "An install is already running");
            return;
        }
        let (tx, rx) = mpsc::channel();
        let paths = self.paths.clone();
        std::thread::spawn(move || {
            let mut sink = ChannelProgress(tx.clone());
            let res = proton::install_dwproton_with(&paths, None, false, &mut sink)
                .map_err(|e| e.to_string());
            let _ = tx.send(TaskMsg::Done(res));
        });
        self.task = Some(InstallTask {
            rx,
            stage: "Starting".into(),
            done: 0,
            total: None,
        });
    }

    /// Drain progress messages from the install thread.
    pub fn poll_task(&mut self) -> Result<()> {
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        let finished = loop {
            match task.rx.try_recv() {
                Ok(TaskMsg::Stage(s)) => {
                    task.stage = s;
                    task.total = None;
                }
                Ok(TaskMsg::DownloadStart { name, total }) => {
                    task.stage = format!("Downloading {name}");
                    task.done = 0;
                    task.total = total;
                }
                Ok(TaskMsg::Advance(n)) => task.done += n,
                Ok(TaskMsg::Done(res)) => break Some(res),
                Err(TryRecvError::Empty) => break None,
                Err(TryRecvError::Disconnected) => {
                    break Some(Err("install thread stopped unexpectedly".into()))
                }
            }
        };
        let Some(res) = finished else {
            return Ok(());
        };
        let already = self
            .task
            .take()
            .is_some_and(|t| t.stage.starts_with("Already installed"));
        self.reload()?;
        match res {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(i) = self.protons.iter().position(|p| p.path == path) {
                    self.proton_sel = i;
                }
                if already {
                    self.notify(ToastKind::Info, format!("{name} is already the latest"));
                } else {
                    self.notify(ToastKind::Ok, format!("Installed {name}"));
                }
            }
            Err(e) => self.notify(ToastKind::Err, format!("dwproton install failed: {e}")),
        }
        Ok(())
    }

    // --- prefixes -------------------------------------------------------------

    pub fn next_prefix_name(&self) -> String {
        (1..)
            .map(|n| format!("pfx-{n}"))
            .find(|id| !self.prefixes.contains(id))
            .unwrap_or_else(|| "pfx".into())
    }

    pub fn create_prefix(&mut self, id: &str) -> Result<Check<()>> {
        let id = id.trim();
        if !is_valid_app_id(id) {
            return Ok(Err("Use lowercase letters, digits and hyphens".into()));
        }
        if self.prefixes.iter().any(|p| p == id) {
            return Ok(Err(format!("`{id}` already exists")));
        }
        let path = self.paths.prefix_path(id);
        launch::ensure_prefix(&path)?;
        self.reload()?;
        if let Some(i) = self.prefixes.iter().position(|p| p == id) {
            self.prefix_sel = i;
        }
        self.notify(ToastKind::Ok, format!("Created {}", path.display()));
        Ok(Ok(()))
    }

    pub fn delete_prefix(&mut self, id: &str) -> Result<()> {
        match apps::delete_prefix(&self.paths, &self.db, id, false) {
            Ok(DeletePrefixOutcome::Removed(p)) => {
                self.notify(ToastKind::Ok, format!("Deleted {}", p.display()))
            }
            Ok(DeletePrefixOutcome::AlreadyGone(p)) => {
                self.notify(ToastKind::Info, format!("Already gone: {}", p.display()))
            }
            Err(e) => self.notify(ToastKind::Err, e.to_string()),
        }
        self.reload()
    }

    pub fn set_default_prefix(&mut self, id: &str) -> Result<()> {
        self.config.default_prefix = id.to_string();
        self.save_config()?;
        self.notify(
            ToastKind::Ok,
            format!("New apps use prefix `{id}` by default"),
        );
        Ok(())
    }

    // --- global settings ------------------------------------------------------

    pub fn set_global_locale(&mut self, idx: usize) -> Result<()> {
        let Some(loc) = locale::selectable().get(idx) else {
            return Ok(());
        };
        self.config.locale = Some(loc.id.to_string());
        self.save_config()?;
        self.notify(
            ToastKind::Ok,
            format!("Locale: {} · {}", loc.label, loc.posix),
        );
        Ok(())
    }

    pub fn cycle_global_locale(&mut self, dir: i32) -> Result<()> {
        let n = locale::selectable().len();
        let cur = locale::selectable_index(self.config.locale.as_deref());
        self.set_global_locale(step(cur, n, dir))
    }

    pub fn toggle_graphics(&mut self) -> Result<()> {
        let next = match config::normalize_graphics(&self.config.launch.graphics) {
            "wined3d" => "dxvk",
            _ => "wined3d",
        };
        self.config.launch.graphics = next.into();
        self.save_config()?;
        self.notify(ToastKind::Ok, format!("Graphics: {next}"));
        Ok(())
    }

    pub fn toggle_sanitize(&mut self) -> Result<()> {
        self.config.launch.sanitize_vulkan = !self.config.launch.sanitize_vulkan;
        self.save_config()?;
        let state = if self.config.launch.sanitize_vulkan {
            "on"
        } else {
            "off"
        };
        self.notify(ToastKind::Ok, format!("Vulkan ICD sanitize: {state}"));
        Ok(())
    }

    pub fn cycle_log_level(&mut self, dir: i32) -> Result<()> {
        const LEVELS: [&str; 3] = ["quiet", "normal", "verbose"];
        let cur = LEVELS
            .iter()
            .position(|l| *l == config::normalize_log_level(&self.config.launch.log_level))
            .unwrap_or(1);
        let next = LEVELS[step(cur, 3, dir)];
        self.config.launch.log_level = next.into();
        self.save_config()?;
        self.notify(ToastKind::Ok, format!("Log level: {next}"));
        Ok(())
    }

    pub fn toggle_gamescope(&mut self) -> Result<()> {
        self.config.launch.gamescope.enabled = !self.config.launch.gamescope.enabled;
        self.save_config()?;
        let label = self.config.launch.gamescope.status_label();
        self.notify(ToastKind::Ok, format!("Gamescope: {label}"));
        Ok(())
    }

    pub fn delete_global_env(&mut self, key: &str) -> Result<()> {
        self.config.launch.env.remove(key);
        self.save_config()?;
        self.notify(ToastKind::Ok, format!("Removed {key}"));
        Ok(())
    }

    pub fn reset_global_env(&mut self) -> Result<()> {
        self.config.launch.env = config::builtin_launch_env();
        self.save_config()?;
        self.notify(ToastKind::Ok, "Environment reset to built-in defaults");
        Ok(())
    }

    pub fn run_doctor(&mut self) -> Result<()> {
        self.reload()?;
        let n = self.issue_count();
        if n == 0 {
            self.notify(ToastKind::Ok, "No problems found");
        } else {
            self.notify(ToastKind::Warn, format!("{n} problem(s) found"));
        }
        Ok(())
    }
}

/// Forwards install progress from the worker thread to the UI.
struct ChannelProgress(Sender<TaskMsg>);

impl InstallProgress for ChannelProgress {
    fn stage(&mut self, msg: &str) {
        let _ = self.0.send(TaskMsg::Stage(msg.to_string()));
    }

    fn download_start(&mut self, name: &str, total: Option<u64>) {
        let _ = self.0.send(TaskMsg::DownloadStart {
            name: name.to_string(),
            total,
        });
    }

    fn download_advance(&mut self, bytes: u64) {
        let _ = self.0.send(TaskMsg::Advance(bytes));
    }

    fn download_finish(&mut self) {}
}
