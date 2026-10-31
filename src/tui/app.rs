//! TUI state: tabs, cached data, overlays, toasts and the background task.
//!
//! Rendering reads only this cached state (no disk I/O per frame); every
//! mutation goes through `actions.rs`, which saves and then reloads.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::apps::{apps_using_prefix, AppEntry, AppsDb};
use crate::config::Config;
use crate::error::Result;
use crate::paths::Paths;
use crate::proton::{self, ProtonInstall};

use super::checks::{self, Check, Level};
use super::overlay::{Overlay, Picker};
use super::text::{fuzzy_best, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Proton,
    Prefixes,
    Settings,
    Doctor,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Library,
        Tab::Proton,
        Tab::Prefixes,
        Tab::Settings,
        Tab::Doctor,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Proton => "Proton",
            Tab::Prefixes => "Prefixes",
            Tab::Settings => "Settings",
            Tab::Doctor => "Doctor",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn step(self, dir: i32) -> Self {
        let n = Self::ALL.len() as i32;
        Self::ALL[(self.index() as i32 + dir).rem_euclid(n) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Warn,
    Err,
}

pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub at: Instant,
}

impl Toast {
    fn ttl(&self) -> Duration {
        match self.kind {
            ToastKind::Info | ToastKind::Ok => Duration::from_secs(4),
            ToastKind::Warn | ToastKind::Err => Duration::from_secs(8),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibFocus {
    List,
    Detail,
}

/// Editable rows of the Library detail pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Exe,
    Proton,
    Locale,
    Graphics,
    Gamescope,
    Env,
}

impl Field {
    pub const ALL: [Field; 7] = [
        Field::Name,
        Field::Exe,
        Field::Proton,
        Field::Locale,
        Field::Graphics,
        Field::Gamescope,
        Field::Env,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Exe => "Exe",
            Field::Proton => "Proton",
            Field::Locale => "Locale",
            Field::Graphics => "Graphics",
            Field::Gamescope => "Gamescope",
            Field::Env => "Env",
        }
    }

    /// Whether ←/→ cycles the value in place.
    pub fn cycles(self) -> bool {
        matches!(
            self,
            Field::Proton | Field::Locale | Field::Graphics | Field::Gamescope
        )
    }

    pub fn help(self) -> &'static str {
        match self {
            Field::Name => "Display name (the id stays fixed)",
            Field::Exe => "After an installer runs, point this at the real game binary",
            Field::Proton => "Proton build for this app; inherit follows the Proton tab default",
            Field::Locale => "LANG / LC_* passed to the game",
            Field::Graphics => "dxvk = Vulkan (default), wined3d = OpenGL fallback",
            Field::Gamescope => "Wrap the game in gamescope (flags come from config.toml)",
            Field::Env => "Extra environment variables layered on the global ones",
        }
    }
}

pub struct Library {
    pub filter: TextInput,
    /// Typing goes to the filter box.
    pub filtering: bool,
    /// Cursor in the *visible* (filtered) list.
    pub sel: usize,
    pub focus: LibFocus,
    /// Cursor in [`Field::ALL`] while the detail pane has focus.
    pub field: usize,
}

/// Global rows on the Settings tab (env rows follow them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Locale,
    Graphics,
    Sanitize,
    LogLevel,
    Gamescope,
}

impl Setting {
    pub const ALL: [Setting; 5] = [
        Setting::Locale,
        Setting::Graphics,
        Setting::Sanitize,
        Setting::LogLevel,
        Setting::Gamescope,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Setting::Locale => "Locale",
            Setting::Graphics => "Graphics",
            Setting::Sanitize => "Vulkan ICD sanitize",
            Setting::LogLevel => "Log level",
            Setting::Gamescope => "Gamescope",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Setting::Locale => "Default LANG / LC_* for apps without an override",
            Setting::Graphics => "dxvk = Direct3D → Vulkan (default), wined3d = OpenGL fallback",
            Setting::Sanitize => {
                "Strip host VK_ICD_FILENAMES / VK_DRIVER_FILES (fixes 32-bit DXVK)"
            }
            Setting::LogLevel => {
                "quiet hides umu/Proton/Wine output, verbose adds protoncli diagnostics"
            }
            Setting::Gamescope => "Wrap every game in gamescope (per-app override in Library)",
        }
    }
}

/// Everything the palette (and some keys) can trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    GoTab(Tab),
    Launch(String),
    AddApp,
    EditApp,
    RemoveApp,
    InstallDwproton,
    UseLatestDwproton,
    TogglePrefer,
    GlobalLocale,
    ToggleGraphics,
    ToggleSanitize,
    CycleLogLevel,
    ToggleGamescope,
    AddGlobalEnv,
    ResetGlobalEnv,
    NewPrefix,
    RunDoctor,
    Help,
    Quit,
}

/// Messages from the dwproton install thread.
pub enum TaskMsg {
    Stage(String),
    DownloadStart { name: String, total: Option<u64> },
    Advance(u64),
    Done(std::result::Result<PathBuf, String>),
}

pub struct InstallTask {
    pub rx: Receiver<TaskMsg>,
    pub stage: String,
    pub done: u64,
    pub total: Option<u64>,
}

impl InstallTask {
    /// 0.0–1.0 while a sized download runs, else `None` (spinner).
    pub fn ratio(&self) -> Option<f64> {
        self.total
            .filter(|t| *t > 0)
            .map(|t| (self.done as f64 / t as f64).clamp(0.0, 1.0))
    }
}

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub db: AppsDb,
    pub tab: Tab,
    pub lib: Library,
    pub protons: Vec<ProtonInstall>,
    pub proton_sel: usize,
    /// Requested Proton name → resolved path (`None` = not found).
    pub proton_resolved: BTreeMap<String, Option<PathBuf>>,
    pub prefixes: Vec<String>,
    pub prefix_sel: usize,
    pub settings_sel: usize,
    pub checks: Vec<Check>,
    pub doctor_scroll: u16,
    pub overlays: Vec<Overlay>,
    pub toast: Option<Toast>,
    pub task: Option<InstallTask>,
    pub tick: u64,
    pub should_quit: bool,
    pub needs_full_redraw: bool,
}

impl App {
    pub fn new(paths: Paths, config: Config) -> Result<Self> {
        let first_run = config.locale.is_none();
        let mut app = Self {
            paths,
            config,
            db: AppsDb::default(),
            tab: Tab::Library,
            lib: Library {
                filter: TextInput::default(),
                filtering: false,
                sel: 0,
                focus: LibFocus::List,
                field: 0,
            },
            protons: Vec::new(),
            proton_sel: 0,
            proton_resolved: BTreeMap::new(),
            prefixes: Vec::new(),
            prefix_sel: 0,
            settings_sel: 0,
            checks: Vec::new(),
            doctor_scroll: 0,
            overlays: Vec::new(),
            toast: None,
            task: None,
            tick: 0,
            should_quit: false,
            needs_full_redraw: false,
        };
        app.reload()?;
        if let Some(i) = app.active_proton_index() {
            app.proton_sel = i;
        }
        if first_run {
            app.overlays
                .push(Overlay::Picker(Picker::first_run_locale()));
        }
        Ok(app)
    }

    /// Re-read apps, Proton installs and prefixes from disk, then re-run checks.
    pub fn reload(&mut self) -> Result<()> {
        self.db = AppsDb::load(&self.paths)?;
        self.protons = proton::list_installs(&self.paths).unwrap_or_default();
        self.prefixes = self.paths.list_prefix_ids()?;
        self.refresh_proton_cache();
        self.checks = checks::build(&self.paths, &self.config, &self.db, &self.proton_resolved);
        self.clamp();
        Ok(())
    }

    fn refresh_proton_cache(&mut self) {
        let mut wanted: Vec<String> = vec![self.config.proton.default.clone()];
        wanted.extend(
            self.db
                .apps
                .values()
                .filter_map(|e| e.proton.clone())
                .filter(|p| !p.is_empty()),
        );
        self.proton_resolved = wanted
            .into_iter()
            .map(|req| {
                let path = proton::resolve_proton_path(&self.paths, &self.config.proton, &req).ok();
                (req, path)
            })
            .collect();
    }

    fn clamp(&mut self) {
        let n = self.visible_apps().len();
        self.lib.sel = self.lib.sel.min(n.saturating_sub(1));
        self.proton_sel = self.proton_sel.min(self.protons.len().saturating_sub(1));
        self.prefix_sel = self.prefix_sel.min(self.prefixes.len().saturating_sub(1));
        self.settings_sel = self.settings_sel.min(self.settings_len().saturating_sub(1));
    }

    pub fn notify(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toast = Some(Toast {
            kind,
            text: text.into(),
            at: Instant::now(),
        });
    }

    pub fn expire_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.at.elapsed() > t.ttl())
        {
            self.toast = None;
        }
    }

    // --- library queries ----------------------------------------------------

    /// App ids in display order: by name, or by match quality while filtering.
    pub fn visible_apps(&self) -> Vec<String> {
        let query = self.lib.filter.trimmed();
        let mut rows: Vec<(usize, String, &str)> = self
            .db
            .apps
            .iter()
            .filter_map(|(id, e)| {
                fuzzy_best(query, &[e.name.as_str(), id.as_str()])
                    .map(|score| (score, e.name.to_lowercase(), id.as_str()))
            })
            .collect();
        if query.is_empty() {
            rows.sort_by(|a, b| a.1.cmp(&b.1));
        } else {
            rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        }
        rows.into_iter().map(|(_, _, id)| id.to_string()).collect()
    }

    pub fn selected_app_id(&self) -> Option<String> {
        self.visible_apps().get(self.lib.sel).cloned()
    }

    pub fn selected_app(&self) -> Option<(String, &AppEntry)> {
        let id = self.selected_app_id()?;
        let entry = self.db.apps.get(&id)?;
        Some((id, entry))
    }

    pub fn select_app(&mut self, id: &str) {
        if let Some(i) = self.visible_apps().iter().position(|a| a == id) {
            self.lib.sel = i;
        }
    }

    pub fn prefix_exists(&self, entry: &AppEntry) -> bool {
        self.paths.prefix_path(&entry.prefix).is_dir()
    }

    pub fn current_field(&self) -> Field {
        Field::ALL[self.lib.field.min(Field::ALL.len() - 1)]
    }

    // --- proton queries -----------------------------------------------------

    pub fn resolved(&self, requested: &str) -> Option<&PathBuf> {
        self.proton_resolved.get(requested).and_then(Option::as_ref)
    }

    /// `"dwproton → dwproton-11.0-15"`, or the name plus `(not found)`.
    pub fn describe_proton(&self, requested: &str) -> String {
        match self.resolved(requested) {
            Some(path) => {
                let dir = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if dir == requested {
                    dir
                } else {
                    format!("{requested} → {dir}")
                }
            }
            None => format!("{requested} (not found)"),
        }
    }

    pub fn active_proton_index(&self) -> Option<usize> {
        let path = self.resolved(&self.config.proton.default)?;
        self.protons.iter().position(|p| &p.path == path)
    }

    /// How many apps launch with this install (overrides + inheriting the default).
    pub fn proton_users(&self, install: &ProtonInstall) -> usize {
        let default_path = self.resolved(&self.config.proton.default);
        self.db
            .apps
            .values()
            .filter(|e| {
                let req =
                    proton::effective_requested(e.proton.as_deref(), &self.config.proton.default);
                let path = if req == self.config.proton.default {
                    default_path
                } else {
                    self.resolved(req)
                };
                path == Some(&install.path)
            })
            .count()
    }

    // --- prefixes / settings / doctor -----------------------------------------

    /// Names of apps whose prefix resolves to this bottle.
    pub fn prefix_users(&self, prefix_id: &str) -> Vec<&str> {
        apps_using_prefix(&self.db, &self.paths, prefix_id)
            .iter()
            .filter_map(|id| self.db.apps.get(id).map(|e| e.name.as_str()))
            .collect()
    }

    pub fn env_keys(&self) -> Vec<String> {
        self.config.launch.env.keys().cloned().collect()
    }

    /// Settings rows: globals, then env pairs, then the "add variable" row.
    pub fn settings_len(&self) -> usize {
        Setting::ALL.len() + self.config.launch.env.len() + 1
    }

    pub fn issue_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| matches!(c.level, Level::Warn | Level::Err))
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.checks.iter().any(|c| c.level == Level::Err)
    }
}
