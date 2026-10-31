//! Minimal setup TUI (launched when protoncli is run with no subcommand).
//!
//! Covers the settings a player typically needs before launching games:
//! app list / launch, register app, wine prefixes, Proton runtime, launch settings
//! (locale / graphics / Vulkan sanitize / log level / gamescope), **environment variables**, doctor.
//! On first run (`config.locale` unset) a locale picker is shown before the home menu.
//! Per-app **proton** / locale / graphics / gamescope / **exe** / **env** can be set from the Apps list (`o` or `e`).
//! Exe change uses a filtered browser (dirs + `.exe` only) over the wineprefix or host FS.

use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;

use crate::apps::{delete_prefix, new_entry, AppsDb, DeletePrefixOutcome};
use crate::browse::{self, BrowseEntry};
use crate::config::{self, Config};
use crate::desktop;
use crate::doctor;
use crate::error::{Error, Result};
use crate::launch;
use crate::locale;
use crate::paths::{is_valid_app_id, slugify, Paths, DEFAULT_PREFIX_ID};
use crate::proton;

/// Rows on the global launch-settings screen.
const LAUNCH_SET_LOCALE: usize = 0;
const LAUNCH_SET_GRAPHICS: usize = 1;
const LAUNCH_SET_SANITIZE: usize = 2;
const LAUNCH_SET_LOG_LEVEL: usize = 3;
const LAUNCH_SET_GAMESCOPE: usize = 4;
const LAUNCH_SET_LEN: usize = 5;

/// Rows on the per-app settings screen.
const APP_SET_NAME: usize = 0;
const APP_SET_EXE: usize = 1;
const APP_SET_PROTON: usize = 2;
const APP_SET_LOCALE: usize = 3;
const APP_SET_GRAPHICS: usize = 4;
const APP_SET_GAMESCOPE: usize = 5;
const APP_SET_ENV: usize = 6;
const APP_SET_LEN: usize = 7;

/// Rows on the “where to browse for exe” chooser.
const EXE_MODE_PREFIX: usize = 0;
const EXE_MODE_FILESYSTEM: usize = 1;
const EXE_MODE_LEN: usize = 2;

/// First row on AppEnv is the Default/Custom mode toggle; env pairs follow.
const APP_ENV_MODE_ROW: usize = 0;

// --- palette ----------------------------------------------------------------
// Body text inherits the terminal default foreground so light *and* dark
// themes stay readable. Never use Color::White as text-only (invisible on
// light backgrounds). Selection / focus use fg+bg pairs for contrast that
// does not depend on the terminal theme.

/// Primary body text — terminal default fg (works on light and dark).
fn s_text() -> Style {
    Style::default()
}
fn s_title() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}
fn s_screen() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}
fn s_dim() -> Style {
    // DarkGray is dark enough on light themes and still secondary on dark.
    // Modifier::DIM alone is unreliable across terminals.
    Style::default().fg(Color::DarkGray)
}
fn s_hint() -> Style {
    Style::default().fg(Color::Blue)
}
fn s_ok() -> Style {
    Style::default().fg(Color::Green)
}
fn s_warn() -> Style {
    // Bold helps yellow survive light backgrounds (bright yellow-on-white is weak).
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}
fn s_accent() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}
fn s_sel() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}
fn s_sel_row() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}
fn s_border() -> Style {
    Style::default().fg(Color::Blue)
}
fn s_border_status() -> Style {
    Style::default().fg(Color::Yellow)
}
fn s_border_warn() -> Style {
    Style::default().fg(Color::Magenta)
}
fn s_field_active() -> Style {
    // Black-on-yellow is high-contrast on both light and dark backgrounds.
    Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}
fn s_field_idle() -> Style {
    Style::default().fg(Color::Gray)
}
/// Focused editable value: yellow on dark-gray island (readable on light themes).
fn s_focus_value() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}
fn s_quit() -> Style {
    Style::default().fg(Color::Red)
}
/// Error / missing resource (e.g. app row whose wineprefix dir is gone).
fn s_err() -> Style {
    Style::default().fg(Color::Red)
}
/// Selected row that is also an error (missing prefix): keep selection visible.
fn s_err_sel() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD)
}
/// Currently configured Proton (not just cursor highlight).
fn s_active_item() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}
/// Cursor is on the currently configured Proton.
fn s_active_sel() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// First-run only: must pick a locale before the rest of the TUI.
    LocaleSetup,
    /// Change global default locale (sub-picker from LaunchSettings).
    Locale,
    /// Global launch options: locale, graphics, Vulkan sanitize, log level.
    LaunchSettings,
    /// Global default environment variables (`config.launch.env`).
    EnvSettings,
    /// Add / edit a single KEY=VALUE (global or per-app custom).
    EnvEdit,
    /// Per-app locale picker (inherit or override).
    AppLocale,
    /// Per-app Proton picker (inherit or a discovered install).
    AppProton,
    /// Per-app launch options: name + exe + proton + locale + graphics + env.
    AppSettings,
    /// Edit display name for the open app.
    AppNameEdit,
    /// Per-app env: Default vs Custom (+ extra KEY=VALUE list).
    AppEnv,
    /// Choose wineprefix vs host filesystem before the exe browser.
    ExeBrowseMode,
    /// Filtered dir browser (directories + `.exe` only).
    ExeBrowser,
    Home,
    Apps,
    AddApp,
    Prefixes,
    Proton,
    Doctor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddField {
    Name,
    Exe,
    Id,
    Prefix,
    Proton,
}

/// Register-app prefix picker: shared default bottle, per-app auto id, or typed custom id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddPrefixMode {
    /// `config.default_prefix` (usually `default` → ~/.protoncli/default).
    Default,
    /// Use the resolved app id as the prefix slug (`prefixes/<id>`).
    Auto,
    /// User-typed relative id (starts empty until they type).
    Custom,
}

/// Why the shared exe browser is open (controls Esc / pick destinations).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowsePurpose {
    /// Apps → settings → change exe.
    AppSettings,
    /// Register app → pick .exe from host filesystem.
    AddApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvEditField {
    Key,
    Value,
}

/// Whether EnvEdit / EnvSettings list targets global or per-app custom env.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvScope {
    Global,
    App,
}

struct App {
    paths: Paths,
    config: Config,
    screen: Screen,
    home_idx: usize,
    apps_state: ListState,
    prefix_state: ListState,
    status: String,
    add_name: String,
    add_exe: String,
    add_id: String,
    /// Prefix mode for Register app (default / auto / custom).
    add_prefix_mode: AddPrefixMode,
    /// Custom prefix id text (only used when [`AddPrefixMode::Custom`]; starts empty).
    add_prefix_custom: String,
    /// Selected index into the register-app Proton list (discovered installs).
    add_proton_sel: usize,
    add_field: AddField,
    app_ids: Vec<String>,
    prefix_ids: Vec<String>,
    /// Discovered Proton installs (name / path / source) for the Proton screen.
    proton_installs: Vec<proton::ProtonInstall>,
    proton_sel: usize,
    /// Who opened the shared exe browser ([`BrowsePurpose`]).
    browse_purpose: BrowsePurpose,
    /// Blinking text cursor for form fields that accept typing.
    cursor_blink_on: bool,
    last_blink: Instant,
    /// Index for global locale picker (`selectable`).
    locale_sel: usize,
    /// Index for per-app locale picker (`app_picker_*`, 0 = inherit).
    app_locale_sel: usize,
    /// App id being edited on AppSettings / AppLocale / exe browser.
    app_settings_id: Option<String>,
    /// Current exe path for the app open in AppSettings (cached display + apply).
    app_settings_exe: PathBuf,
    /// Display name while AppSettings / AppNameEdit is open.
    app_settings_name: String,
    /// Selected row on [`Screen::LaunchSettings`].
    launch_set_sel: usize,
    /// Selected row on [`Screen::AppSettings`].
    app_set_sel: usize,
    /// Per-app graphics picker index (0 = inherit, 1 = dxvk, 2 = wined3d).
    app_graphics_sel: usize,
    /// Per-app gamescope picker index (0 = inherit, 1 = on, 2 = off).
    app_gamescope_sel: usize,
    /// Per-app Proton picker index (0 = inherit, 1+ = [`Self::proton_installs`]).
    app_proton_sel: usize,
    /// Selected row on global EnvSettings (index into sorted keys).
    env_sel: usize,
    /// Selected row on AppEnv (0 = mode toggle; 1+ = custom env keys).
    app_env_sel: usize,
    /// Cached `use_custom_env` while AppEnv / AppSettings is open.
    app_use_custom_env: bool,
    /// Scope for EnvEdit and for which map a/d operate on.
    env_scope: EnvScope,
    env_edit_key: String,
    env_edit_value: String,
    env_edit_field: EnvEditField,
    /// Previous key when editing an existing entry (rename removes the old key).
    env_edit_original_key: Option<String>,
    /// Selected row on [`Screen::ExeBrowseMode`].
    exe_browse_mode_sel: usize,
    /// Current directory in [`Screen::ExeBrowser`].
    browse_cwd: PathBuf,
    /// Do not navigate above this path (wineprefix root); `None` = unrestricted host FS.
    browse_floor: Option<PathBuf>,
    /// Listing for `browse_cwd` (dirs + `.exe` only). Synthetic `..` is not stored here.
    browse_entries: Vec<BrowseEntry>,
    browse_state: ListState,
    /// True when the list includes a leading `..` row.
    browse_has_parent: bool,
    doctor_lines: Vec<String>,
    should_quit: bool,
    /// After leaving the alternate screen (game launch / installer), force a
    /// full terminal clear so ratatui does not differential-draw against a
    /// blank alt-screen with a stale internal buffer.
    needs_full_redraw: bool,
}

const HOME_ITEMS: &[&str] = &[
    "Apps — list / launch",
    "Register app",
    "Wine prefixes",
    "Proton settings",
    "Launch settings (locale / graphics / gamescope)",
    "Environment variables",
    "Doctor (health check)",
    "Quit",
];

fn app_graphics_picker_index(app: Option<&str>) -> usize {
    match app {
        None => 0,
        Some("wined3d") => 2,
        Some(_) => 1, // dxvk or unknown → dxvk row
    }
}

fn app_graphics_picker_value(sel: usize) -> Option<&'static str> {
    match sel {
        0 => None,
        1 => Some("dxvk"),
        2 => Some("wined3d"),
        _ => None,
    }
}

fn app_graphics_picker_label(i: usize, global: &str) -> String {
    let g = config::normalize_graphics(global);
    match i {
        0 => format!("Use global default  [{g}]"),
        1 => "dxvk  —  Direct3D → Vulkan (default, like Steam)".into(),
        2 => "wined3d  —  Direct3D → OpenGL (fallback if DXVK fails)".into(),
        _ => "(invalid)".into(),
    }
}

fn app_gamescope_picker_index(app: Option<bool>) -> usize {
    match app {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }
}

fn app_gamescope_picker_value(sel: usize) -> Option<bool> {
    match sel {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn app_gamescope_picker_label(i: usize, global: &config::GamescopeConfig) -> String {
    match i {
        0 => format!("Use global default  [{}]", global.status_label()),
        1 => "on  —  wrap with gamescope (host FS + force nested FS)".into(),
        2 => "off  —  plain umu/Proton (no gamescope)".into(),
        _ => "(invalid)".into(),
    }
}

/// Rows in the per-app Proton picker: inherit + each discovered install.
fn app_proton_picker_len(installs: &[proton::ProtonInstall]) -> usize {
    1 + installs.len()
}

/// Map `app.proton` to picker index (`0` = inherit global).
fn app_proton_picker_index(app: Option<&str>, installs: &[proton::ProtonInstall]) -> usize {
    match app {
        None => 0,
        Some(name) => installs
            .iter()
            .position(|p| p.name == name)
            .map(|i| i + 1)
            .unwrap_or(0),
    }
}

/// Map picker index back to `Option` Proton name (`None` = inherit).
fn app_proton_picker_value(
    sel: usize,
    installs: &[proton::ProtonInstall],
) -> Option<String> {
    if sel == 0 {
        None
    } else {
        installs.get(sel - 1).map(|p| p.name.clone())
    }
}

pub fn run(paths: Paths, config: Config) -> Result<()> {
    // release は panic = "abort" なので後片付けのコードは走らない。
    // パニック時も端末を raw mode / 代替スクリーンのまま残さないよう hook で戻す。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_raw_mode().ok();
        execute!(stdout(), LeaveAlternateScreen, Show).ok();
        default_hook(info);
    }));

    enable_raw_mode().map_err(Error::Io)?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).map_err(Error::Io)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).map_err(Error::Io)?;

    let mut app = App::new(paths, config)?;
    let result = app_loop(&mut terminal, &mut app);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

impl App {
    fn new(paths: Paths, config: Config) -> Result<Self> {
        // First TUI open (or upgraded config without locale): force a choice
        // before the home menu so Japanese titles/paths get a sane LANG early.
        let needs_locale = config.locale.is_none();
        let locale_sel = locale::selectable_index(config.locale.as_deref());
        let mut app = Self {
            paths,
            config,
            screen: if needs_locale {
                Screen::LocaleSetup
            } else {
                Screen::Home
            },
            home_idx: 0,
            apps_state: ListState::default(),
            prefix_state: ListState::default(),
            status: if needs_locale {
                "First run: choose a launch locale  ↑↓  Enter confirm".into()
            } else {
                "↑↓ move  Enter select  Esc back  q quit".into()
            },
            add_name: String::new(),
            add_exe: String::new(),
            add_id: String::new(),
            add_prefix_mode: AddPrefixMode::Default,
            add_prefix_custom: String::new(),
            add_proton_sel: 0,
            add_field: AddField::Name,
            app_ids: Vec::new(),
            prefix_ids: Vec::new(),
            proton_installs: Vec::new(),
            proton_sel: 0,
            browse_purpose: BrowsePurpose::AppSettings,
            cursor_blink_on: true,
            last_blink: Instant::now(),
            locale_sel,
            app_locale_sel: 0,
            app_settings_id: None,
            app_settings_exe: PathBuf::new(),
            app_settings_name: String::new(),
            launch_set_sel: 0,
            app_set_sel: 0,
            app_graphics_sel: 0,
            app_gamescope_sel: 0,
            app_proton_sel: 0,
            env_sel: 0,
            app_env_sel: 0,
            app_use_custom_env: false,
            env_scope: EnvScope::Global,
            env_edit_key: String::new(),
            env_edit_value: String::new(),
            env_edit_field: EnvEditField::Key,
            env_edit_original_key: None,
            exe_browse_mode_sel: 0,
            browse_cwd: PathBuf::new(),
            browse_floor: None,
            browse_entries: Vec::new(),
            browse_state: ListState::default(),
            browse_has_parent: false,
            doctor_lines: Vec::new(),
            should_quit: false,
            needs_full_redraw: false,
        };
        app.refresh_lists()?;
        Ok(app)
    }

    fn apply_global_locale(&mut self) -> Result<()> {
        let list = locale::selectable();
        let Some(loc) = list.get(self.locale_sel) else {
            self.status = "no locale selected".into();
            return Ok(());
        };
        self.config.locale = Some(loc.id.to_string());
        self.config.save(&self.paths)?;
        self.status = format!(
            "global locale = {} ({}) — apps without override inherit this",
            loc.id, loc.posix
        );
        Ok(())
    }

    fn cycle_global_graphics(&mut self) -> Result<()> {
        let next = match config::normalize_graphics(&self.config.launch.graphics) {
            "wined3d" => "dxvk",
            _ => "wined3d",
        };
        self.config.launch.graphics = next.into();
        self.config.save(&self.paths)?;
        self.status = format!("global graphics = {next}");
        Ok(())
    }

    fn toggle_sanitize_vulkan(&mut self) -> Result<()> {
        self.config.launch.sanitize_vulkan = !self.config.launch.sanitize_vulkan;
        self.config.save(&self.paths)?;
        self.status = format!(
            "sanitize_vulkan = {} (strip host VK_ICD_FILENAMES)",
            if self.config.launch.sanitize_vulkan {
                "ON"
            } else {
                "OFF"
            }
        );
        Ok(())
    }

    fn cycle_log_level(&mut self) -> Result<()> {
        let next = config::next_log_level(&self.config.launch.log_level);
        self.config.launch.log_level = next.into();
        self.config.save(&self.paths)?;
        let hint = match next {
            "quiet" => "hide umu/Proton/Wine terminal output",
            "verbose" => "inherit streams + extra protoncli diagnostics",
            _ => "show umu/Proton/Wine output (default)",
        };
        self.status = format!("log_level = {next} ({hint})");
        Ok(())
    }

    fn toggle_global_gamescope(&mut self) -> Result<()> {
        // Only the master switch; width / -f / extra_args stay in config.toml.
        self.config.launch.gamescope.enabled = !self.config.launch.gamescope.enabled;
        self.config.save(&self.paths)?;
        self.status = format!(
            "gamescope = {} (default flags: -f --force-windows-fullscreen)",
            self.config.launch.gamescope.status_label()
        );
        Ok(())
    }

    fn apply_app_gamescope(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let value = app_gamescope_picker_value(self.app_gamescope_sel);
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.gamescope = value;
        db.save(&self.paths)?;
        self.status = match value {
            Some(true) => format!("`{id}` gamescope = on (per-app override)"),
            Some(false) => format!("`{id}` gamescope = off (per-app override)"),
            None => format!("`{id}` gamescope = inherit global"),
        };
        Ok(())
    }

    fn apply_app_locale(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let value = locale::app_picker_value(self.app_locale_sel).map(str::to_string);
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.locale = value.clone();
        db.save(&self.paths)?;
        self.status = match value {
            Some(loc) => format!("`{id}` locale = {loc} (per-app override)"),
            None => format!("`{id}` locale = inherit global"),
        };
        Ok(())
    }

    fn apply_app_proton(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let value = app_proton_picker_value(self.app_proton_sel, &self.proton_installs);
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.proton = value.clone();
        db.save(&self.paths)?;
        self.status = match value {
            Some(p) => format!("`{id}` proton = {p} (per-app override)"),
            None => format!(
                "`{id}` proton = inherit global [{}]",
                self.config.proton.default
            ),
        };
        Ok(())
    }

    fn apply_app_graphics(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let value = app_graphics_picker_value(self.app_graphics_sel).map(str::to_string);
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.graphics = value.clone();
        db.save(&self.paths)?;
        self.status = match value {
            Some(g) => format!("`{id}` graphics = {g} (per-app override)"),
            None => format!("`{id}` graphics = inherit global"),
        };
        Ok(())
    }

    fn open_app_settings(&mut self) -> Result<()> {
        let Some(i) = self.apps_state.selected() else {
            self.status = "select an app first".into();
            return Ok(());
        };
        let Some(id) = self.app_ids.get(i).cloned() else {
            self.status = "select an app first".into();
            return Ok(());
        };
        let db = AppsDb::load(&self.paths)?;
        let entry = db.get(&id)?;
        // Refresh so the Proton picker sees installs added since TUI start.
        self.proton_installs = proton::list_installs(&self.paths).unwrap_or_default();
        self.app_locale_sel = locale::app_picker_index(entry.locale.as_deref());
        self.app_graphics_sel = app_graphics_picker_index(entry.graphics.as_deref());
        self.app_gamescope_sel = app_gamescope_picker_index(entry.gamescope);
        self.app_proton_sel =
            app_proton_picker_index(entry.proton.as_deref(), &self.proton_installs);
        self.app_use_custom_env = entry.use_custom_env;
        self.app_settings_exe = entry.exe.clone();
        self.app_settings_name = entry.name.clone();
        self.app_settings_id = Some(id.clone());
        self.app_set_sel = 0;
        self.status = format!(
            "settings for `{id}`  ↑↓/jk/C-n C-p  Enter edit  Esc back"
        );
        self.screen = Screen::AppSettings;
        Ok(())
    }

    fn open_app_name_edit(&mut self) {
        let id = self.app_settings_id.as_deref().unwrap_or("?");
        self.status = format!("edit name for `{id}`  Enter save  Esc cancel");
        self.screen = Screen::AppNameEdit;
    }

    fn apply_app_name(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let name = self.app_settings_name.trim().to_string();
        if name.is_empty() {
            self.status = "name required".into();
            return Ok(());
        }
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.name = name.clone();
        let desktop = entry.desktop;
        let entry_snapshot = entry.clone();
        db.save(&self.paths)?;
        if desktop {
            if let Err(e) =
                desktop::install_shortcut(&self.paths, &self.config.desktop, &id, &entry_snapshot)
            {
                self.status = format!("name updated for `{id}` but desktop refresh failed: {e}");
                self.screen = Screen::AppSettings;
                return Ok(());
            }
        }
        self.app_settings_name = name.clone();
        self.status = format!("`{id}` name → {name}");
        self.screen = Screen::AppSettings;
        Ok(())
    }

    fn open_global_env_settings(&mut self) {
        self.env_scope = EnvScope::Global;
        self.env_sel = 0;
        self.status =
            "Global env  ↑↓  Enter/e edit  a add  d del  r reset defaults  Esc back".into();
        self.screen = Screen::EnvSettings;
    }

    fn open_app_env_settings(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let db = AppsDb::load(&self.paths)?;
        let entry = db.get(&id)?;
        self.app_use_custom_env = entry.use_custom_env;
        self.env_scope = EnvScope::App;
        self.app_env_sel = 0;
        self.status = format!(
            "env for `{id}`  ↑↓  Enter  a add  d del  Esc back (custom layers on defaults)"
        );
        self.screen = Screen::AppEnv;
        Ok(())
    }

    /// Sorted keys for the map currently edited by env screens.
    fn env_keys_for_scope(&self) -> Result<Vec<String>> {
        match self.env_scope {
            EnvScope::Global => Ok(self.config.launch.env.keys().cloned().collect()),
            EnvScope::App => {
                let Some(id) = self.app_settings_id.as_deref() else {
                    return Ok(Vec::new());
                };
                let db = AppsDb::load(&self.paths)?;
                let entry = db.get(id)?;
                Ok(entry.env.keys().cloned().collect())
            }
        }
    }

    fn toggle_app_custom_env(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        entry.use_custom_env = !entry.use_custom_env;
        self.app_use_custom_env = entry.use_custom_env;
        let mode = if entry.use_custom_env {
            "custom (defaults + extras; extras override)"
        } else {
            "default (global [launch.env] only)"
        };
        db.save(&self.paths)?;
        self.status = format!("`{id}` env mode = {mode}");
        Ok(())
    }

    fn start_env_edit_new(&mut self) {
        self.env_edit_key.clear();
        self.env_edit_value.clear();
        self.env_edit_field = EnvEditField::Key;
        self.env_edit_original_key = None;
        self.status = "New env  Tab field  Enter save  Esc cancel".into();
        self.screen = Screen::EnvEdit;
    }

    fn start_env_edit_existing(&mut self, key: &str, value: &str) {
        self.env_edit_key = key.to_string();
        self.env_edit_value = value.to_string();
        self.env_edit_field = EnvEditField::Value;
        self.env_edit_original_key = Some(key.to_string());
        self.status = format!("Edit `{key}`  Tab field  Enter save  Esc cancel");
        self.screen = Screen::EnvEdit;
    }

    fn save_env_edit(&mut self) -> Result<()> {
        let key = self.env_edit_key.trim().to_string();
        if key.is_empty() {
            self.status = "env key required".into();
            return Ok(());
        }
        if key.contains('=') || key.contains('\0') || key.contains('\n') {
            self.status = "invalid env key (no '=' or control chars)".into();
            return Ok(());
        }
        let value = self.env_edit_value.clone();
        let original = self.env_edit_original_key.clone();

        match self.env_scope {
            EnvScope::Global => {
                if let Some(ref old) = original {
                    if old != &key {
                        self.config.launch.env.remove(old);
                    }
                }
                self.config.launch.env.insert(key.clone(), value);
                self.config.save(&self.paths)?;
                // Keep selection on the saved key.
                let keys: Vec<_> = self.config.launch.env.keys().cloned().collect();
                self.env_sel = keys.iter().position(|k| k == &key).unwrap_or(0);
                self.status = format!("global env `{key}` saved");
                self.screen = Screen::EnvSettings;
            }
            EnvScope::App => {
                let Some(id) = self.app_settings_id.clone() else {
                    self.status = "no app selected".into();
                    return Ok(());
                };
                let mut db = AppsDb::load(&self.paths)?;
                {
                    let entry = db
                        .apps
                        .get_mut(&id)
                        .ok_or_else(|| Error::AppNotFound(id.clone()))?;
                    // Editing extras implies custom mode so the value is actually applied.
                    entry.use_custom_env = true;
                    if let Some(ref old) = original {
                        if old != &key {
                            entry.env.remove(old);
                        }
                    }
                    entry.env.insert(key.clone(), value.clone());
                }
                self.app_use_custom_env = true;
                db.save(&self.paths)?;
                let keys: Vec<_> = db
                    .apps
                    .get(&id)
                    .map(|e| e.env.keys().cloned().collect())
                    .unwrap_or_default();
                // app_env_sel: 0 = mode, 1+ = keys
                self.app_env_sel = keys
                    .iter()
                    .position(|k| k == &key)
                    .map(|i| i + 1)
                    .unwrap_or(0);
                self.status = format!("`{id}` custom env `{key}` saved (layers on defaults)");
                self.screen = Screen::AppEnv;
            }
        }
        Ok(())
    }

    fn delete_env_at_selection(&mut self) -> Result<()> {
        match self.env_scope {
            EnvScope::Global => {
                let keys: Vec<_> = self.config.launch.env.keys().cloned().collect();
                if keys.is_empty() {
                    self.status = "no global env entries".into();
                    return Ok(());
                }
                let i = self.env_sel.min(keys.len() - 1);
                let key = keys[i].clone();
                self.config.launch.env.remove(&key);
                self.config.save(&self.paths)?;
                if self.env_sel > 0 && self.env_sel >= self.config.launch.env.len() {
                    self.env_sel = self.env_sel.saturating_sub(1);
                }
                self.status = format!("removed global env `{key}`");
            }
            EnvScope::App => {
                let Some(id) = self.app_settings_id.clone() else {
                    self.status = "no app selected".into();
                    return Ok(());
                };
                if self.app_env_sel == APP_ENV_MODE_ROW {
                    self.status = "select an env row to delete (not the mode row)".into();
                    return Ok(());
                }
                let mut db = AppsDb::load(&self.paths)?;
                let keys: Vec<_> = db
                    .apps
                    .get(&id)
                    .map(|e| e.env.keys().cloned().collect())
                    .unwrap_or_default();
                let ki = self.app_env_sel - 1;
                if ki >= keys.len() {
                    self.status = "no env entry selected".into();
                    return Ok(());
                }
                let key = keys[ki].clone();
                let n = {
                    let entry = db
                        .apps
                        .get_mut(&id)
                        .ok_or_else(|| Error::AppNotFound(id.clone()))?;
                    entry.env.remove(&key);
                    entry.env.len()
                };
                db.save(&self.paths)?;
                // Clamp: mode + n keys (last index is n when n keys remain)
                let max = n;
                if self.app_env_sel > max {
                    self.app_env_sel = max;
                }
                self.status = format!("removed `{id}` env `{key}`");
            }
        }
        Ok(())
    }

    fn reset_global_env_defaults(&mut self) -> Result<()> {
        self.config.launch.env = config::builtin_launch_env();
        self.config.save(&self.paths)?;
        self.env_sel = 0;
        self.status =
            "global env reset to built-ins (DXVK_HUD=0, STEAMOS=1, UMU_USE_STEAM=1)".into();
        Ok(())
    }

    fn env_value_for_scope(&self, key: &str) -> Result<Option<String>> {
        match self.env_scope {
            EnvScope::Global => Ok(self.config.launch.env.get(key).cloned()),
            EnvScope::App => {
                let Some(id) = self.app_settings_id.as_deref() else {
                    return Ok(None);
                };
                let db = AppsDb::load(&self.paths)?;
                Ok(db.get(id)?.env.get(key).cloned())
            }
        }
    }

    fn leave_env_edit(&mut self) {
        match self.env_scope {
            EnvScope::Global => {
                self.status = "Global env  ↑↓  Enter/e edit  a add  d del  r reset  Esc".into();
                self.screen = Screen::EnvSettings;
            }
            EnvScope::App => {
                let id = self.app_settings_id.as_deref().unwrap_or("?");
                self.status = format!("env for `{id}`  ↑↓  Enter  a add  d del  Esc");
                self.screen = Screen::AppEnv;
            }
        }
    }

    /// After installers land files in the bottle, point the registered app at the real exe.
    fn apply_app_exe(&mut self, new_exe: PathBuf) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let exe = if new_exe.is_absolute() {
            new_exe
        } else {
            std::env::current_dir()?.join(&new_exe)
        };
        if !exe.is_file() {
            self.status = format!("exe not found: {}", exe.display());
            return Ok(());
        }
        if !browse::is_windows_exe(&exe) {
            self.status = format!("not a .exe: {}", exe.display());
            return Ok(());
        }

        let mut db = AppsDb::load(&self.paths)?;
        let entry = db
            .apps
            .get_mut(&id)
            .ok_or_else(|| Error::AppNotFound(id.clone()))?;
        // Keep launch cwd next to the binary (same rule as initial registration).
        entry.cwd = exe.parent().map(Path::to_path_buf);
        entry.exe = exe.clone();
        let desktop = entry.desktop;
        let entry_snapshot = entry.clone();
        db.save(&self.paths)?;

        if desktop {
            match desktop::install_shortcut(&self.paths, &self.config.desktop, &id, &entry_snapshot)
            {
                Ok(_) => {}
                Err(e) => {
                    self.app_settings_exe = exe;
                    self.status = format!("exe updated for `{id}` but desktop refresh failed: {e}");
                    return Ok(());
                }
            }
        }

        self.app_settings_exe = exe.clone();
        self.status = format!("`{id}` exe → {}", exe.display());
        Ok(())
    }

    fn open_exe_browse_mode(&mut self) {
        self.browse_purpose = BrowsePurpose::AppSettings;
        self.exe_browse_mode_sel = 0;
        let id = self.app_settings_id.as_deref().unwrap_or("?");
        self.status = format!("change exe for `{id}`  ↑↓  Enter  Esc back");
        self.screen = Screen::ExeBrowseMode;
    }

    /// Start filtered browser under the app wineprefix (capped at that root).
    fn start_browse_prefix(&mut self) -> Result<()> {
        let Some(id) = self.app_settings_id.clone() else {
            self.status = "no app selected".into();
            return Ok(());
        };
        let db = AppsDb::load(&self.paths)?;
        let entry = db.get(&id)?;
        let prefix = self.paths.prefix_path(&entry.prefix);
        if !prefix.is_dir() {
            self.status = format!(
                "prefix not found: {} — run the installer once to create it",
                prefix.display()
            );
            return Ok(());
        }
        self.browse_purpose = BrowsePurpose::AppSettings;
        self.browse_floor = Some(prefix.clone());
        self.browse_cwd = browse::start_dir_for_prefix(&prefix);
        self.reload_browse_entries()?;
        self.status =
            "prefix browse  ↑↓  Enter open/select  Esc mode  (dirs + .exe only)".into();
        self.screen = Screen::ExeBrowser;
        Ok(())
    }

    /// Start filtered browser on the host filesystem (no floor).
    fn start_browse_filesystem(&mut self) -> Result<()> {
        self.browse_floor = None;
        let start = match self.browse_purpose {
            BrowsePurpose::AppSettings => {
                browse::start_dir_for_filesystem(&self.app_settings_exe)
            }
            // Register form: seed from the path already picked, else $HOME.
            BrowsePurpose::AddApp => {
                browse::start_dir_for_filesystem(Path::new(self.add_exe.trim()))
            }
        };
        self.browse_cwd = start;
        self.reload_browse_entries()?;
        self.status = match self.browse_purpose {
            BrowsePurpose::AppSettings => {
                "filesystem browse  ↑↓  Enter open/select  Esc mode  (dirs + .exe only)".into()
            }
            BrowsePurpose::AddApp => {
                "pick .exe  ↑↓  Enter open/select  Esc back to register  (dirs + .exe only)"
                    .into()
            }
        };
        self.screen = Screen::ExeBrowser;
        Ok(())
    }

    /// Open host-FS exe picker from Register app (no wineprefix yet).
    fn start_add_app_exe_browser(&mut self) -> Result<()> {
        self.browse_purpose = BrowsePurpose::AddApp;
        self.start_browse_filesystem()
    }

    /// Register-form Proton starts at inherit (row 0) so new apps follow global default.
    fn reset_add_proton_sel(&mut self) {
        self.add_proton_sel = 0;
    }

    /// Toggle form text cursor visibility (~2 Hz when the active field accepts typing).
    fn tick_cursor_blink(&mut self) {
        if !self.add_field_is_text_input() && self.screen != Screen::AppNameEdit {
            // Env edit also types; keep blink there too.
            if self.screen != Screen::EnvEdit {
                return;
            }
        }
        if self.last_blink.elapsed() >= Duration::from_millis(500) {
            self.cursor_blink_on = !self.cursor_blink_on;
            self.last_blink = Instant::now();
        }
    }

    fn reset_cursor_blink(&mut self) {
        self.cursor_blink_on = true;
        self.last_blink = Instant::now();
    }

    /// Fields that accept free-text typing on the Register form.
    fn add_field_is_text_input(&self) -> bool {
        match self.add_field {
            AddField::Name | AddField::Id => true,
            AddField::Prefix => self.add_prefix_mode == AddPrefixMode::Custom,
            AddField::Exe | AddField::Proton => false,
        }
    }

    fn reload_browse_entries(&mut self) -> Result<()> {
        self.browse_has_parent =
            browse::can_go_up(&self.browse_cwd, self.browse_floor.as_deref());
        self.browse_entries = browse::list_exe_browser(&self.browse_cwd)?;
        let len = self.browse_list_len();
        if len == 0 {
            self.browse_state.select(None);
        } else {
            let sel = self.browse_state.selected().unwrap_or(0).min(len - 1);
            self.browse_state.select(Some(sel));
        }
        Ok(())
    }

    fn browse_list_len(&self) -> usize {
        self.browse_entries.len() + usize::from(self.browse_has_parent)
    }

    /// Resolve list index to parent-up or a concrete entry.
    fn browse_item_at(&self, idx: usize) -> Option<BrowseItem<'_>> {
        if self.browse_has_parent {
            if idx == 0 {
                return Some(BrowseItem::Parent);
            }
            return self
                .browse_entries
                .get(idx - 1)
                .map(BrowseItem::Entry);
        }
        self.browse_entries.get(idx).map(BrowseItem::Entry)
    }

    fn refresh_lists(&mut self) -> Result<()> {
        let db = AppsDb::load(&self.paths)?;
        self.app_ids = db.apps.keys().cloned().collect();
        if self.apps_state.selected().is_none() && !self.app_ids.is_empty() {
            self.apps_state.select(Some(0));
        }
        if let Some(i) = self.apps_state.selected() {
            if i >= self.app_ids.len() {
                self.apps_state
                    .select(if self.app_ids.is_empty() {
                        None
                    } else {
                        Some(self.app_ids.len() - 1)
                    });
            }
        }

        self.prefix_ids = self.paths.list_prefix_ids()?;
        if self.prefix_state.selected().is_none() && !self.prefix_ids.is_empty() {
            self.prefix_state.select(Some(0));
        }

        self.proton_installs = proton::list_installs(&self.paths).unwrap_or_default();
        // Keep cursor in range; prefer the currently resolved default when possible.
        if self.proton_installs.is_empty() {
            self.proton_sel = 0;
        } else if self.proton_sel >= self.proton_installs.len() {
            self.proton_sel = self
                .active_proton_index()
                .unwrap_or(0)
                .min(self.proton_installs.len() - 1);
        }
        Ok(())
    }

    /// Index of the install that matches the configured / resolved default Proton.
    fn active_proton_index(&self) -> Option<usize> {
        let resolved =
            proton::resolve_proton_path(&self.paths, &self.config.proton, &self.config.proton.default)
                .ok();
        if let Some(path) = resolved.as_ref() {
            if let Some(i) = self.proton_installs.iter().position(|p| &p.path == path) {
                return Some(i);
            }
        }
        // Fallback: exact name match against config.proton.default
        let def = self.config.proton.default.as_str();
        self.proton_installs.iter().position(|p| p.name == def)
    }

    /// True when this install is the one currently used as the global default.
    fn is_active_proton(&self, install: &proton::ProtonInstall) -> bool {
        self.active_proton_index()
            .and_then(|i| self.proton_installs.get(i))
            .is_some_and(|p| p.path == install.path)
    }

    /// Set global default Proton to the install under the cursor (and align prefer).
    fn set_default_proton_from_sel(&mut self) -> Result<()> {
        let Some(inst) = self.proton_installs.get(self.proton_sel).cloned() else {
            self.status = "no Proton selected — install with [i] or refresh [r]".into();
            return Ok(());
        };
        // Name match is how resolve_proton_path picks installs; prefer matches source
        // so "dwproton" / same-named tools resolve to the intended tree first.
        self.config.proton.default = inst.name.clone();
        self.config.proton.prefer = inst.source.to_string();
        // List selection is the source of truth — drop any absolute override.
        self.config.proton.explicit_path = None;
        self.config.save(&self.paths)?;
        self.status = format!(
            "default Proton = {} [{}]  prefer={}",
            inst.name, inst.source, inst.source
        );
        Ok(())
    }

    fn refresh_doctor(&mut self) {
        let report = doctor::run_doctor(&self.paths, &self.config);
        let mut lines = Vec::new();
        lines.push(format!("root      : {}", self.paths.root.display()));
        lines.push(format!(
            "default   : {}",
            self.paths.default_prefix.display()
        ));
        lines.push(format!(
            "prefixes  : {}",
            self.paths.prefixes_dir.display()
        ));
        match &report.umu {
            doctor::UmuStatus::Found(p) => lines.push(format!("umu-run   : OK ({})", p.display())),
            doctor::UmuStatus::Missing => {
                lines.push("umu-run   : missing (optional if Steam ready)".into())
            }
        }
        match &report.steam {
            doctor::SteamStatus::Ready {
                client,
                runtime,
                container,
            } => lines.push(format!(
                "Steam     : OK {} / {} (container={})",
                client.display(),
                runtime,
                container
            )),
            doctor::SteamStatus::ClientOnly { client } => {
                lines.push(format!("Steam     : client only ({})", client.display()))
            }
            doctor::SteamStatus::Missing => lines.push("Steam     : missing".into()),
        }
        match &report.backend {
            Some(crate::launch::LaunchBackend::Umu { .. }) => {
                lines.push("backend   : umu".into())
            }
            Some(crate::launch::LaunchBackend::Steam { use_container, .. }) => {
                lines.push(format!("backend   : steam (container={use_container})"))
            }
            None => lines.push("backend   : NONE".into()),
        }
        match &report.default_proton {
            Some(p) => lines.push(format!("PROTONPATH: {}", p.display())),
            None => lines.push("PROTONPATH: unresolved — install dwproton / Steam Proton".into()),
        }
        lines.push(format!(
            "prefer    : {} / default={}",
            self.config.proton.prefer, self.config.proton.default
        ));
        lines.push(format!(
            "default_prefix (config): {}",
            self.config.default_prefix
        ));
        match self.config.locale.as_deref().and_then(locale::find) {
            Some(l) => lines.push(format!("locale    : {} ({}) [global]", l.id, l.posix)),
            None => lines.push("locale    : (not set — open Launch settings)".into()),
        }
        lines.push(format!(
            "graphics  : {} [global]",
            config::normalize_graphics(&self.config.launch.graphics)
        ));
        lines.push(format!(
            "sanitize_vulkan: {} (host VK_ICD strip)",
            if self.config.launch.sanitize_vulkan {
                "ON"
            } else {
                "OFF"
            }
        ));
        lines.push(format!(
            "log_level : {} (quiet hides child stdout/stderr)",
            config::normalize_log_level(&self.config.launch.log_level)
        ));
        lines.push(format!(
            "gamescope : {} [global]",
            self.config.launch.gamescope.status_label()
        ));
        if report.protons.is_empty() {
            lines.push("Proton    : none".into());
        } else {
            lines.push("Proton    : (★ = default)".into());
            for p in &report.protons {
                let mark = report
                    .default_proton
                    .as_ref()
                    .is_some_and(|d| d == &p.path);
                lines.push(format!(
                    "  {}[{}] {}",
                    if mark { "★ " } else { "  " },
                    p.source,
                    p.name
                ));
            }
        }
        self.doctor_lines = lines;
    }
}

/// One navigable row in the exe browser list widget.
enum BrowseItem<'a> {
    Parent,
    Entry(&'a BrowseEntry),
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Resume path: alt-screen is empty but ratatui still believes the last
        // pre-suspend frame is painted. Differential draw then only touches
        // cells that changed (often just the status line) → blank frozen TUI.
        if app.needs_full_redraw {
            terminal.clear().map_err(Error::Io)?;
            // Keys / resize events queued while umu/Proton owned the tty.
            drain_pending_events();
            app.needs_full_redraw = false;
        }

        // Blink text cursor while a form field is accepting typing (poll period ~200ms).
        app.tick_cursor_blink();

        terminal.draw(|f| ui(f, app)).map_err(Error::Io)?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(200)).map_err(Error::Io)? {
            match event::read().map_err(Error::Io)? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    handle_key(app, key)?;
                }
                // Fullscreen games often change the PTY size; force full paint.
                Event::Resize(_, _) => {
                    app.needs_full_redraw = true;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Drop any events left in the crossterm queue (non-blocking).
fn drain_pending_events() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match app.screen {
        Screen::LocaleSetup | Screen::Locale | Screen::AppLocale => handle_locale_key(app, key)?,
        Screen::AppProton => handle_app_proton_key(app, key)?,
        Screen::LaunchSettings => handle_launch_settings_key(app, key)?,
        Screen::EnvSettings => handle_env_settings_key(app, key)?,
        Screen::EnvEdit => handle_env_edit_key(app, key)?,
        Screen::AppEnv => handle_app_env_key(app, key)?,
        Screen::AppSettings => handle_app_settings_key(app, key)?,
        Screen::AppNameEdit => handle_app_name_edit_key(app, key)?,
        Screen::ExeBrowseMode => handle_exe_browse_mode_key(app, key)?,
        Screen::ExeBrowser => handle_exe_browser_key(app, key)?,
        Screen::Home => {
            if is_nav_prev(key, NavMode::List) {
                if app.home_idx > 0 {
                    app.home_idx -= 1;
                }
            } else if is_nav_next(key, NavMode::List) {
                if app.home_idx + 1 < HOME_ITEMS.len() {
                    app.home_idx += 1;
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Enter => match app.home_idx {
                        0 => {
                            app.refresh_lists()?;
                            app.screen = Screen::Apps;
                            app.status =
                                "Enter/l launch · o/e settings · d remove · Esc · nav: jk/C-n C-p"
                                    .into();
                        }
                        1 => {
                            app.add_name.clear();
                            app.add_exe.clear();
                            app.add_id.clear();
                            app.add_prefix_mode = AddPrefixMode::Default;
                            app.add_prefix_custom.clear();
                            app.add_field = AddField::Name;
                            app.refresh_lists()?;
                            app.reset_add_proton_sel();
                            app.reset_cursor_blink();
                            app.screen = Screen::AddApp;
                            app.status = "Tab fields · Enter action (exe pick / prefix cycle / save) · Esc"
                                .into();
                        }
                        2 => {
                            app.refresh_lists()?;
                            app.screen = Screen::Prefixes;
                            app.status =
                                "c create · d delete · s set default · Esc · nav: jk/C-n C-p"
                                    .into();
                        }
                        3 => {
                            app.refresh_lists()?;
                            app.screen = Screen::Proton;
                            app.refresh_lists()?;
                            if let Some(i) = app.active_proton_index() {
                                app.proton_sel = i;
                            }
                            app.status =
                                "↑↓/jk/C-n  Enter set default  [i] install  [m]/[t] prefer  [d] [r]"
                                    .into();
                        }
                        4 => {
                            app.launch_set_sel = 0;
                            app.status =
                                "Launch settings  ↑↓/jk/C-n C-p  Enter edit  Esc back".into();
                            app.screen = Screen::LaunchSettings;
                        }
                        5 => app.open_global_env_settings(),
                        6 => {
                            app.refresh_doctor();
                            app.screen = Screen::Doctor;
                            app.status = "r refresh · Esc back".into();
                        }
                        7 => app.should_quit = true,
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        Screen::Apps => {
            if is_nav_prev(key, NavMode::List) {
                select_prev(&mut app.apps_state, app.app_ids.len());
            } else if is_nav_next(key, NavMode::List) {
                select_next(&mut app.apps_state, app.app_ids.len());
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.screen = Screen::Home;
                        app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
                    }
                    KeyCode::Enter | KeyCode::Char('l') => {
                        if let Some(i) = app.apps_state.selected() {
                            if let Some(id) = app.app_ids.get(i).cloned() {
                                let paths = app.paths.clone();
                                let config = app.config.clone();
                                let launch_res = suspend_tui_for_launch(|| {
                                    let db = AppsDb::load(&paths)?;
                                    let entry = db.get(&id)?;
                                    let spec = launch::LaunchSpec::from_app(&paths, &config, entry)?;
                                    let _ = launch::run_umu(&config, &spec)?;
                                    Ok(())
                                });
                                // Always full-redraw after suspend, including launch errors:
                                // alt-screen was blanked regardless of whether the game ran.
                                app.needs_full_redraw = true;
                                match launch_res {
                                    Ok(()) => {
                                        app.status = format!("launched `{id}` (returned)");
                                    }
                                    Err(e) => {
                                        app.status = format!("launch `{id}` failed: {e}");
                                    }
                                }
                            }
                        }
                    }
                    // `e` is an alias for settings (mnemonic: edit / env); same as `o`.
                    KeyCode::Char('o') | KeyCode::Char('e') => app.open_app_settings()?,
                    KeyCode::Char('d') => {
                        if let Some(i) = app.apps_state.selected() {
                            if let Some(id) = app.app_ids.get(i).cloned() {
                                let mut db = AppsDb::load(&app.paths)?;
                                let entry = db.remove(&id)?;
                                db.save(&app.paths)?;
                                if entry.desktop {
                                    let _ = desktop::uninstall_shortcut(&app.paths, &id);
                                }
                                app.status = format!("removed app `{id}` (prefix kept)");
                                app.refresh_lists()?;
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        app.refresh_lists()?;
                        app.status = "apps refreshed".into();
                    }
                    _ => {}
                }
            }
        }
        Screen::AddApp => {
            // Form: Ctrl+n/p (and Tab/arrows) change fields; bare j/k type into text fields.
            if is_nav_next(key, NavMode::Form) {
                // On Proton field, Down alone cycles the install list (list UX).
                if app.add_field == AddField::Proton
                    && matches!(key.code, KeyCode::Down)
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    cycle_add_proton(app, 1);
                } else {
                    app.add_field = match app.add_field {
                        AddField::Name => AddField::Exe,
                        AddField::Exe => AddField::Id,
                        AddField::Id => AddField::Prefix,
                        AddField::Prefix => AddField::Proton,
                        AddField::Proton => AddField::Name,
                    };
                    app.reset_cursor_blink();
                }
            } else if is_nav_prev(key, NavMode::Form) {
                if app.add_field == AddField::Proton
                    && matches!(key.code, KeyCode::Up)
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    cycle_add_proton(app, -1);
                } else {
                    app.add_field = match app.add_field {
                        AddField::Name => AddField::Proton,
                        AddField::Exe => AddField::Name,
                        AddField::Id => AddField::Exe,
                        AddField::Prefix => AddField::Id,
                        AddField::Proton => AddField::Prefix,
                    };
                    app.reset_cursor_blink();
                }
            } else {
                match key.code {
                    KeyCode::Esc => {
                        app.screen = Screen::Home;
                        app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
                    }
                    // Proton list: j/k or ←→ also cycle (Tab/C-n still leave the field).
                    KeyCode::Char('j')
                        if app.add_field == AddField::Proton
                            && key.modifiers.is_empty() =>
                    {
                        cycle_add_proton(app, 1);
                    }
                    KeyCode::Char('k')
                        if app.add_field == AddField::Proton
                            && key.modifiers.is_empty() =>
                    {
                        cycle_add_proton(app, -1);
                    }
                    KeyCode::Left | KeyCode::Right
                        if app.add_field == AddField::Proton =>
                    {
                        let dir = if key.code == KeyCode::Right { 1 } else { -1 };
                        cycle_add_proton(app, dir);
                    }
                    KeyCode::Char(c)
                        if !c.is_control()
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                            && app.add_field_is_text_input() =>
                    {
                        if let Some(buf) = add_text_field_mut(app) {
                            buf.push(c);
                            app.reset_cursor_blink();
                        }
                    }
                    KeyCode::Backspace if app.add_field_is_text_input() => {
                        if let Some(buf) = add_text_field_mut(app) {
                            buf.pop();
                            app.reset_cursor_blink();
                        }
                    }
                    // Space is typed into text fields; only non-text rows treat Space as activate.
                    KeyCode::Enter => {
                        handle_add_app_activate(app, key.code)?;
                    }
                    KeyCode::Char(' ') if !app.add_field_is_text_input() => {
                        handle_add_app_activate(app, key.code)?;
                    }
                    _ => {}
                }
            }
        }
        Screen::Prefixes => {
            if is_nav_prev(key, NavMode::List) {
                select_prev(&mut app.prefix_state, app.prefix_ids.len());
            } else if is_nav_next(key, NavMode::List) {
                select_next(&mut app.prefix_state, app.prefix_ids.len());
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.screen = Screen::Home;
                        app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
                    }
                    KeyCode::Char('c') => {
                        let mut n = 1u32;
                        loop {
                            let id = format!("pfx-{n}");
                            if !app.prefix_ids.iter().any(|p| p == &id) {
                                let path = app.paths.prefix_path(&id);
                                launch::ensure_prefix(&path)?;
                                app.status = format!("created {}", path.display());
                                break;
                            }
                            n += 1;
                            if n > 999 {
                                app.status = "too many auto prefixes".into();
                                break;
                            }
                        }
                        app.refresh_lists()?;
                    }
                    // Delete managed bottle; no --force in TUI (use CLI if apps still reference it).
                    KeyCode::Char('d') => {
                        if let Some(i) = app.prefix_state.selected() {
                            if let Some(id) = app.prefix_ids.get(i).cloned() {
                                let db = AppsDb::load(&app.paths)?;
                                match delete_prefix(&app.paths, &db, &id, false) {
                                    Ok(DeletePrefixOutcome::Removed(path)) => {
                                        app.status = format!(
                                            "removed prefix `{id}`  ({})",
                                            path.display()
                                        );
                                        app.refresh_lists()?;
                                    }
                                    Ok(DeletePrefixOutcome::AlreadyGone(path)) => {
                                        app.status = format!(
                                            "prefix `{id}` already gone  ({})",
                                            path.display()
                                        );
                                        app.refresh_lists()?;
                                    }
                                    Err(e) => {
                                        app.status = format!("{e}");
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(i) = app.prefix_state.selected() {
                            if let Some(id) = app.prefix_ids.get(i).cloned() {
                                app.config.default_prefix = id.clone();
                                app.config.save(&app.paths)?;
                                app.status = format!("default_prefix = `{id}`");
                            }
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(i) = app.prefix_state.selected() {
                            if let Some(id) = app.prefix_ids.get(i) {
                                let path = app.paths.prefix_path(id);
                                app.status = format!("{id} → {}", path.display());
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        app.refresh_lists()?;
                    }
                    _ => {}
                }
            }
        }
        Screen::Proton => {
            if is_nav_prev(key, NavMode::List) {
                cycle_prev(&mut app.proton_sel, app.proton_installs.len());
            } else if is_nav_next(key, NavMode::List) {
                cycle_next(&mut app.proton_sel, app.proton_installs.len());
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.screen = Screen::Home;
                        app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
                    }
                    // Primary action: pick the highlighted install as the global default.
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        app.set_default_proton_from_sel()?;
                    }
                    KeyCode::Char('i') => {
                        let paths = app.paths.clone();
                        let install_res = suspend_tui_for_launch(|| {
                            println!("Installing dwproton…");
                            let path = proton::install_dwproton(&paths, None, false)?;
                            println!("Installed: {}", path.display());
                            Ok(())
                        });
                        app.needs_full_redraw = true;
                        match install_res {
                            Ok(()) => {
                                app.refresh_lists()?;
                                // Jump cursor to the newly preferred managed dwproton if present.
                                if let Some(i) = app.active_proton_index() {
                                    app.proton_sel = i;
                                }
                                app.status = "dwproton install finished".into();
                            }
                            Err(e) => {
                                app.status = format!("dwproton install failed: {e}");
                            }
                        }
                    }
                    KeyCode::Char('m') => {
                        app.config.proton.prefer = "managed".into();
                        app.config.save(&app.paths)?;
                        app.status =
                            "prefer = managed (used when default is logical dwproton)".into();
                    }
                    KeyCode::Char('t') => {
                        app.config.proton.prefer = "steam".into();
                        app.config.save(&app.paths)?;
                        app.status =
                            "prefer = steam (used when default is logical dwproton)".into();
                    }
                    KeyCode::Char('d') => {
                        // Logical alias: newest dwproton-* with prefer ranking.
                        app.config.proton.default = "dwproton".into();
                        app.config.proton.explicit_path = None;
                        app.config.save(&app.paths)?;
                        if let Some(i) = app.active_proton_index() {
                            app.proton_sel = i;
                        }
                        app.status = "default proton = dwproton (latest matching prefer)".into();
                    }
                    KeyCode::Char('r') => {
                        app.refresh_lists()?;
                        if let Some(i) = app.active_proton_index() {
                            app.proton_sel = i;
                        }
                        app.status = "proton list refreshed".into();
                    }
                    _ => {}
                }
            }
        }
        Screen::Doctor => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Home;
                app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
            }
            KeyCode::Char('r') => {
                app.refresh_doctor();
                app.status = "doctor refreshed".into();
            }
            _ => {}
        },
    }
    Ok(())
}

/// List/menu vs text-form field movement.
///
/// Forms keep bare `j`/`k` as typed characters; lists use them as vim motions.
/// Both accept **Ctrl+n / Ctrl+p** (and Ctrl+j / Ctrl+k) so Register App never
/// needs the arrow keys.
#[derive(Clone, Copy)]
enum NavMode {
    List,
    Form,
}

fn is_nav_next(key: KeyEvent, mode: NavMode) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Down => true,
        KeyCode::Tab if matches!(mode, NavMode::Form) => true,
        KeyCode::Char('n' | 'j') if ctrl => true,
        KeyCode::Char('j') if matches!(mode, NavMode::List) && key.modifiers.is_empty() => true,
        _ => false,
    }
}

fn is_nav_prev(key: KeyEvent, mode: NavMode) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => true,
        KeyCode::BackTab if matches!(mode, NavMode::Form) => true,
        KeyCode::Char('p' | 'k') if ctrl => true,
        KeyCode::Char('k') if matches!(mode, NavMode::List) && key.modifiers.is_empty() => true,
        _ => false,
    }
}

fn cycle_prev(sel: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    *sel = if *sel == 0 { len - 1 } else { *sel - 1 };
}

fn cycle_next(sel: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    *sel = (*sel + 1) % len;
}

fn handle_locale_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let first_run = app.screen == Screen::LocaleSetup;
    let per_app = app.screen == Screen::AppLocale;
    let n = if per_app {
        locale::app_picker_len()
    } else {
        locale::selectable().len()
    };

    if is_nav_prev(key, NavMode::List) {
        if n > 0 {
            let sel = if per_app {
                &mut app.app_locale_sel
            } else {
                &mut app.locale_sel
            };
            cycle_prev(sel, n);
        }
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        if n > 0 {
            let sel = if per_app {
                &mut app.app_locale_sel
            } else {
                &mut app.locale_sel
            };
            cycle_next(sel, n);
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Esc if !first_run => {
            if per_app {
                app.screen = Screen::AppSettings;
                app.status = "↑↓/jk/C-n C-p row  Enter edit  Esc back".into();
            } else {
                // Global locale picker is nested under Launch settings.
                app.screen = Screen::LaunchSettings;
                app.status = "Launch settings  ↑↓/jk/C-n C-p  Enter edit  Esc back".into();
            }
        }
        KeyCode::Char('q') if first_run => {
            app.should_quit = true;
        }
        KeyCode::Char('q') if !first_run && !per_app => {
            app.screen = Screen::LaunchSettings;
        }
        KeyCode::Char('q') if per_app => {
            app.screen = Screen::AppSettings;
        }
        KeyCode::Enter => {
            if per_app {
                app.apply_app_locale()?;
                app.screen = Screen::AppSettings;
            } else {
                app.apply_global_locale()?;
                if first_run {
                    app.screen = Screen::Home;
                } else {
                    app.screen = Screen::LaunchSettings;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_launch_settings_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if is_nav_prev(key, NavMode::List) {
        cycle_prev(&mut app.launch_set_sel, LAUNCH_SET_LEN);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        cycle_next(&mut app.launch_set_sel, LAUNCH_SET_LEN);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::Home;
            app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.launch_set_sel {
            LAUNCH_SET_LOCALE => {
                app.locale_sel = locale::selectable_index(app.config.locale.as_deref());
                app.status = "Global default locale  ↑↓/jk/C-n C-p  Enter save  Esc back".into();
                app.screen = Screen::Locale;
            }
            LAUNCH_SET_GRAPHICS => app.cycle_global_graphics()?,
            LAUNCH_SET_SANITIZE => app.toggle_sanitize_vulkan()?,
            LAUNCH_SET_LOG_LEVEL => app.cycle_log_level()?,
            LAUNCH_SET_GAMESCOPE => app.toggle_global_gamescope()?,
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_app_settings_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if is_nav_prev(key, NavMode::List) {
        cycle_prev(&mut app.app_set_sel, APP_SET_LEN);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        cycle_next(&mut app.app_set_sel, APP_SET_LEN);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.app_settings_id = None;
            app.screen = Screen::Apps;
            app.status = "Enter/l launch · o/e settings · d remove · Esc · nav: jk/C-n C-p".into();
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.app_set_sel {
            APP_SET_NAME => app.open_app_name_edit(),
            APP_SET_EXE => app.open_exe_browse_mode(),
            APP_SET_PROTON => {
                let id = app.app_settings_id.as_deref().unwrap_or("?");
                let n = app_proton_picker_len(&app.proton_installs);
                if n > 0 && app.app_proton_sel >= n {
                    app.app_proton_sel = 0;
                }
                app.status = format!(
                    "proton for `{id}`  ↑↓/jk/C-n C-p  Enter save  Esc back"
                );
                app.screen = Screen::AppProton;
            }
            APP_SET_LOCALE => {
                let id = app.app_settings_id.as_deref().unwrap_or("?");
                app.status = format!("locale for `{id}`  ↑↓/jk/C-n C-p  Enter save  Esc back");
                app.screen = Screen::AppLocale;
            }
            APP_SET_GRAPHICS => {
                // Cycle inherit → dxvk → wined3d → inherit.
                app.app_graphics_sel = (app.app_graphics_sel + 1) % 3;
                app.apply_app_graphics()?;
            }
            APP_SET_GAMESCOPE => {
                // Cycle inherit → on → off → inherit.
                app.app_gamescope_sel = (app.app_gamescope_sel + 1) % 3;
                app.apply_app_gamescope()?;
            }
            APP_SET_ENV => app.open_app_env_settings()?,
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_app_proton_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let n = app_proton_picker_len(&app.proton_installs).max(1);
    if is_nav_prev(key, NavMode::List) {
        cycle_prev(&mut app.app_proton_sel, n);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        cycle_next(&mut app.app_proton_sel, n);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Reload index from disk so cancel discards an unsaved cursor move.
            if let Some(id) = app.app_settings_id.as_deref() {
                if let Ok(db) = AppsDb::load(&app.paths) {
                    if let Ok(entry) = db.get(id) {
                        app.app_proton_sel = app_proton_picker_index(
                            entry.proton.as_deref(),
                            &app.proton_installs,
                        );
                    }
                }
            }
            let id = app.app_settings_id.as_deref().unwrap_or("?");
            app.status = format!("settings for `{id}`  ↑↓/jk/C-n C-p  Enter edit  Esc back");
            app.screen = Screen::AppSettings;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.apply_app_proton()?;
            app.screen = Screen::AppSettings;
        }
        _ => {}
    }
    Ok(())
}

fn handle_app_name_edit_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            // Reload cached name from disk so cancel discards edits.
            if let Some(id) = app.app_settings_id.as_deref() {
                if let Ok(db) = AppsDb::load(&app.paths) {
                    if let Ok(entry) = db.get(id) {
                        app.app_settings_name = entry.name.clone();
                    }
                }
            }
            let id = app.app_settings_id.as_deref().unwrap_or("?");
            app.status = format!("settings for `{id}`  ↑↓/jk/C-n C-p  Enter edit  Esc back");
            app.screen = Screen::AppSettings;
        }
        KeyCode::Enter => app.apply_app_name()?,
        KeyCode::Backspace => {
            app.app_settings_name.pop();
        }
        KeyCode::Char(c)
            if !c.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.app_settings_name.push(c);
        }
        _ => {}
    }
    Ok(())
}

fn handle_env_settings_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let keys = app.env_keys_for_scope()?;
    let n = keys.len();
    if is_nav_prev(key, NavMode::List) {
        if n > 0 {
            cycle_prev(&mut app.env_sel, n);
        }
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        if n > 0 {
            cycle_next(&mut app.env_sel, n);
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::Home;
            app.status = "↑↓/jk/C-n C-p move  Enter select  Esc back  q quit".into();
        }
        KeyCode::Char('a') => {
            app.env_scope = EnvScope::Global;
            app.start_env_edit_new();
        }
        KeyCode::Char('r') => app.reset_global_env_defaults()?,
        KeyCode::Char('d') => app.delete_env_at_selection()?,
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') => {
            if n == 0 {
                app.env_scope = EnvScope::Global;
                app.start_env_edit_new();
            } else {
                let i = app.env_sel.min(n - 1);
                let key_name = keys[i].clone();
                if let Some(val) = app.env_value_for_scope(&key_name)? {
                    app.env_scope = EnvScope::Global;
                    app.start_env_edit_existing(&key_name, &val);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_app_env_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let keys = app.env_keys_for_scope()?;
    // Rows: [mode] + keys when custom, or just [mode] when default.
    let list_len = if app.app_use_custom_env {
        1 + keys.len()
    } else {
        1
    };
    if is_nav_prev(key, NavMode::List) {
        cycle_prev(&mut app.app_env_sel, list_len);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        cycle_next(&mut app.app_env_sel, list_len);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            let id = app.app_settings_id.as_deref().unwrap_or("?");
            app.status = format!("settings for `{id}`  ↑↓/jk/C-n C-p  Enter edit  Esc back");
            app.screen = Screen::AppSettings;
        }
        KeyCode::Char('a') => {
            // Adding always switches to custom so the pair is applied at launch.
            app.env_scope = EnvScope::App;
            app.start_env_edit_new();
        }
        KeyCode::Char('d') => {
            app.env_scope = EnvScope::App;
            app.delete_env_at_selection()?;
        }
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') => {
            if app.app_env_sel == APP_ENV_MODE_ROW {
                app.toggle_app_custom_env()?;
                // Leaving custom → clamp selection to mode row.
                if !app.app_use_custom_env {
                    app.app_env_sel = 0;
                }
            } else if app.app_use_custom_env {
                let ki = app.app_env_sel - 1;
                if let Some(key_name) = keys.get(ki).cloned() {
                    if let Some(val) = app.env_value_for_scope(&key_name)? {
                        app.env_scope = EnvScope::App;
                        app.start_env_edit_existing(&key_name, &val);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_env_edit_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if is_nav_next(key, NavMode::Form) || is_nav_prev(key, NavMode::Form) {
        app.env_edit_field = match app.env_edit_field {
            EnvEditField::Key => EnvEditField::Value,
            EnvEditField::Value => EnvEditField::Key,
        };
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => app.leave_env_edit(),
        KeyCode::Enter => app.save_env_edit()?,
        KeyCode::Backspace => match app.env_edit_field {
            EnvEditField::Key => {
                app.env_edit_key.pop();
            }
            EnvEditField::Value => {
                app.env_edit_value.pop();
            }
        },
        KeyCode::Char(c)
            if !c.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            match app.env_edit_field {
                EnvEditField::Key => app.env_edit_key.push(c),
                EnvEditField::Value => app.env_edit_value.push(c),
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_exe_browse_mode_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Register form never opens this mode (goes straight to host FS).
    if is_nav_prev(key, NavMode::List) {
        cycle_prev(&mut app.exe_browse_mode_sel, EXE_MODE_LEN);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        cycle_next(&mut app.exe_browse_mode_sel, EXE_MODE_LEN);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            let id = app.app_settings_id.as_deref().unwrap_or("?");
            app.status = format!("settings for `{id}`  ↑↓/jk/C-n C-p  Enter edit  Esc back");
            app.screen = Screen::AppSettings;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.browse_purpose = BrowsePurpose::AppSettings;
            match app.exe_browse_mode_sel {
                EXE_MODE_PREFIX => app.start_browse_prefix()?,
                EXE_MODE_FILESYSTEM => app.start_browse_filesystem()?,
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_exe_browser_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let len = app.browse_list_len();
    if is_nav_prev(key, NavMode::List) {
        select_prev(&mut app.browse_state, len);
        return Ok(());
    }
    if is_nav_next(key, NavMode::List) {
        select_next(&mut app.browse_state, len);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => match app.browse_purpose {
            BrowsePurpose::AppSettings => app.open_exe_browse_mode(),
            BrowsePurpose::AddApp => {
                app.screen = Screen::AddApp;
                app.add_field = AddField::Exe;
                app.status =
                    "exe picker cancelled  · Enter pick again · Tab fields · Enter save on Name"
                        .into();
            }
        },
        KeyCode::Enter | KeyCode::Char(' ') => {
            let Some(idx) = app.browse_state.selected() else {
                app.status = "empty directory (Esc back)".into();
                return Ok(());
            };
            // Clone paths out of the borrow before mutating app.
            let action = match app.browse_item_at(idx) {
                Some(BrowseItem::Parent) => BrowseAction::Up,
                Some(BrowseItem::Entry(e)) if e.is_dir => BrowseAction::EnterDir(e.path.clone()),
                Some(BrowseItem::Entry(e)) => BrowseAction::PickExe(e.path.clone()),
                None => return Ok(()),
            };
            match action {
                BrowseAction::Up => {
                    if let Some(parent) = app.browse_cwd.parent() {
                        app.browse_cwd = parent.to_path_buf();
                        app.reload_browse_entries()?;
                    }
                }
                BrowseAction::EnterDir(path) => {
                    app.browse_cwd = path;
                    app.browse_state.select(Some(0));
                    app.reload_browse_entries()?;
                }
                BrowseAction::PickExe(path) => match app.browse_purpose {
                    BrowsePurpose::AppSettings => {
                        app.apply_app_exe(path)?;
                        // Status already set by apply; append navigation hint on success.
                        if !app.status.contains("failed")
                            && !app.status.contains("not found")
                            && !app.status.contains("not a .exe")
                        {
                            app.status = format!("{}  · Esc Apps", app.status);
                        }
                        app.screen = Screen::AppSettings;
                    }
                    BrowsePurpose::AddApp => {
                        if !path.is_file() {
                            app.status = format!("exe not found: {}", path.display());
                            return Ok(());
                        }
                        if !browse::is_windows_exe(&path) {
                            app.status = format!("not a .exe: {}", path.display());
                            return Ok(());
                        }
                        app.add_exe = path.display().to_string();
                        app.add_field = AddField::Id;
                        app.reset_cursor_blink();
                        app.status = format!(
                            "exe = {}  · Tab fields · Enter save on Name/Id",
                            path.display()
                        );
                        app.screen = Screen::AddApp;
                    }
                },
            }
        }
        // Quick jump: home directory (filesystem mode only — prefix floor ignores this).
        KeyCode::Char('~') | KeyCode::Char('h') if app.browse_floor.is_none() => {
            if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
                app.browse_cwd = home;
                app.browse_state.select(Some(0));
                app.reload_browse_entries()?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Enter/Space on a Register-app field: pick / cycle / save depending on the row.
fn handle_add_app_activate(app: &mut App, code: KeyCode) -> Result<()> {
    let is_space = matches!(code, KeyCode::Char(' '));
    match app.add_field {
        // Built-in host FS picker (prefix may not exist yet at register time).
        AddField::Exe => {
            app.start_add_app_exe_browser()?;
        }
        // Enter/Space cycles default → auto → custom (custom starts with empty text).
        AddField::Prefix => {
            app.add_prefix_mode = match app.add_prefix_mode {
                AddPrefixMode::Default => AddPrefixMode::Auto,
                AddPrefixMode::Auto => AddPrefixMode::Custom,
                AddPrefixMode::Custom => AddPrefixMode::Default,
            };
            app.reset_cursor_blink();
            app.status = match app.add_prefix_mode {
                AddPrefixMode::Default => {
                    "prefix = default (shared ~/.protoncli/default)".into()
                }
                AddPrefixMode::Auto => "prefix = auto (same slug as app id)".into(),
                AddPrefixMode::Custom => "prefix = custom (type a relative id)".into(),
            };
        }
        // Space cycles Proton list; Enter saves the form with the selected install.
        AddField::Proton if is_space => {
            cycle_add_proton(app, 1);
        }
        AddField::Name | AddField::Id | AddField::Proton => {
            save_new_app(app)?;
        }
    }
    Ok(())
}

fn cycle_add_proton(app: &mut App, dir: i32) {
    // Row 0 is inherit; remaining rows are discovered installs.
    let n = app_proton_picker_len(&app.proton_installs).max(1);
    let cur = app.add_proton_sel.min(n - 1) as i32;
    let next = (cur + dir).rem_euclid(n as i32) as usize;
    app.add_proton_sel = next;
    if next == 0 {
        app.status = format!(
            "proton = inherit global [{}]",
            app.config.proton.default
        );
    } else if let Some(p) = app.proton_installs.get(next - 1) {
        app.status = format!("proton = {} [{}]", p.name, p.source);
    }
}

/// Resolved app id for the register form (explicit or slug from name/exe).
fn add_resolved_id(app: &App) -> String {
    if !app.add_id.trim().is_empty() {
        return app.add_id.trim().to_string();
    }
    let name = {
        let n = app.add_name.trim();
        if n.is_empty() {
            crate::paths::name_from_exe(Path::new(app.add_exe.trim()))
        } else {
            n.to_string()
        }
    };
    slugify(&name)
}

/// Prefix id that will be written on save for the current mode.
fn add_resolved_prefix(app: &App) -> std::result::Result<String, String> {
    match app.add_prefix_mode {
        AddPrefixMode::Default => Ok(app.config.default_prefix.clone()),
        AddPrefixMode::Auto => Ok(add_resolved_id(app)),
        AddPrefixMode::Custom => {
            let p = app.add_prefix_custom.trim();
            if p.is_empty() {
                return Err("custom prefix is empty — type an id or switch mode".into());
            }
            Ok(p.to_string())
        }
    }
}

fn add_resolved_proton(app: &App) -> Option<String> {
    app_proton_picker_value(app.add_proton_sel, &app.proton_installs)
}


enum BrowseAction {
    Up,
    EnterDir(PathBuf),
    PickExe(PathBuf),
}

fn save_new_app(app: &mut App) -> Result<()> {
    let exe = PathBuf::from(app.add_exe.trim());
    if app.add_exe.trim().is_empty() {
        app.status = "exe is required — focus Exe and Enter to pick".into();
        return Ok(());
    }
    // Match CLI: empty name → stem of the exe (editable later in App settings).
    let name = {
        let n = app.add_name.trim();
        if n.is_empty() {
            crate::paths::name_from_exe(&exe)
        } else {
            n.to_string()
        }
    };
    let id = add_resolved_id(app);
    if !is_valid_app_id(&id) {
        app.status = format!("invalid id `{id}`");
        return Ok(());
    }
    let prefix = match add_resolved_prefix(app) {
        Ok(p) => p,
        Err(msg) => {
            app.status = msg;
            return Ok(());
        }
    };
    if !PathBuf::from(&prefix).is_absolute()
        && prefix != DEFAULT_PREFIX_ID
        && !is_valid_app_id(&prefix)
    {
        app.status = format!("invalid prefix id `{prefix}`");
        return Ok(());
    }
    let proton = add_resolved_proton(app);
    let create_desktop = app.config.desktop.create_on_add;
    match new_entry(
        name,
        exe,
        vec![],
        None,
        prefix.clone(),
        proton,
        None,
        None,
        create_desktop,
    ) {
        Ok(entry) => {
            let mut db = AppsDb::load(&app.paths)?;
            if let Err(e) = db.insert(id.clone(), entry.clone()) {
                app.status = format!("{e}");
                return Ok(());
            }
            db.save(&app.paths)?;
            launch::ensure_prefix(&app.paths.prefix_path(&prefix))?;
            if create_desktop {
                match desktop::install_shortcut(&app.paths, &app.config.desktop, &id, &entry) {
                    Ok(p) => {
                        app.status = format!(
                            "registered `{id}` → {} | desktop {}",
                            app.paths.prefix_path(&prefix).display(),
                            p.display()
                        );
                    }
                    Err(e) => app.status = format!("registered but desktop failed: {e}"),
                }
            } else {
                app.status = format!(
                    "registered `{id}` → {}",
                    app.paths.prefix_path(&prefix).display()
                );
            }
            app.refresh_lists()?;
            app.screen = Screen::Apps;
        }
        Err(e) => app.status = format!("{e}"),
    }
    Ok(())
}

/// Mutable buffer for the active register-form text field, if any.
fn add_text_field_mut(app: &mut App) -> Option<&mut String> {
    match app.add_field {
        AddField::Name => Some(&mut app.add_name),
        AddField::Id => Some(&mut app.add_id),
        AddField::Prefix if app.add_prefix_mode == AddPrefixMode::Custom => {
            Some(&mut app.add_prefix_custom)
        }
        AddField::Exe | AddField::Prefix | AddField::Proton => None,
    }
}

fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state.selected().unwrap_or(0);
    state.select(Some(if i == 0 { len - 1 } else { i - 1 }));
}

fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = state.selected().unwrap_or(0);
    state.select(Some(if i + 1 >= len { 0 } else { i + 1 }));
}

/// Drop alt-screen / raw mode, run action, then restore for continued TUI use.
///
/// Caller must set [`App::needs_full_redraw`] after this returns so the next
/// frame clears ratatui's stale buffer (see `app_loop`).
fn suspend_tui_for_launch(action: impl FnOnce() -> Result<()>) -> Result<()> {
    disable_raw_mode().ok();
    let mut out = stdout();
    // Show cursor while the external process owns the main screen.
    execute!(out, LeaveAlternateScreen, Show).ok();
    let res = action();
    // Re-enter alt-screen first, then raw mode — same order as `run()`.
    execute!(out, EnterAlternateScreen, Hide).map_err(Error::Io)?;
    enable_raw_mode().map_err(Error::Io)?;
    res
}

fn block_main(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(s_border())
        .title(Span::styled(title.to_string(), s_title()))
}

fn block_warn(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(s_border_warn())
        .title(Span::styled(title.to_string(), s_screen()))
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(f.area());

    let screen_label = match app.screen {
        Screen::LocaleSetup => "Locale setup",
        Screen::Locale => "Locale (global)",
        Screen::LaunchSettings => "Launch settings",
        Screen::EnvSettings => "Environment",
        Screen::EnvEdit => "Edit env",
        Screen::AppLocale => "Locale (app)",
        Screen::AppProton => "Proton (app)",
        Screen::AppSettings => "App settings",
        Screen::AppNameEdit => "Edit name",
        Screen::AppEnv => "App env",
        Screen::ExeBrowseMode => "Exe source",
        Screen::ExeBrowser => "Exe browser",
        Screen::Home => "Home",
        Screen::Apps => "Apps",
        Screen::AddApp => "Register",
        Screen::Prefixes => "Prefixes",
        Screen::Proton => "Proton",
        Screen::Doctor => "Doctor",
    };

    let title = Paragraph::new(Line::from(vec![
        Span::styled(" protoncli ", s_title()),
        Span::styled(format!(" root={} ", app.paths.root.display()), s_dim()),
        Span::styled(screen_label, s_screen()),
        Span::raw("  "),
        match app.config.locale.as_deref().and_then(locale::find) {
            Some(l) => Span::styled(format!("global={}", l.id), s_ok()),
            None => Span::styled("global=?", s_warn()),
        },
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(s_border())
            .title(Span::styled("setup TUI", s_title())),
    );
    f.render_widget(title, chunks[0]);

    match app.screen {
        Screen::LocaleSetup | Screen::Locale | Screen::AppLocale => {
            ui_locale_picker(f, app, chunks[1]);
        }
        Screen::AppProton => ui_app_proton(f, app, chunks[1]),
        Screen::LaunchSettings => ui_launch_settings(f, app, chunks[1]),
        Screen::EnvSettings => ui_env_settings(f, app, chunks[1]),
        Screen::EnvEdit => ui_env_edit(f, app, chunks[1]),
        Screen::AppSettings => ui_app_settings(f, app, chunks[1]),
        Screen::AppNameEdit => ui_app_name_edit(f, app, chunks[1]),
        Screen::AppEnv => ui_app_env(f, app, chunks[1]),
        Screen::ExeBrowseMode => ui_exe_browse_mode(f, app, chunks[1]),
        Screen::ExeBrowser => ui_exe_browser(f, app, chunks[1]),
        Screen::Home => {
            let items: Vec<ListItem> = HOME_ITEMS
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let selected = i == app.home_idx;
                    let is_quit = i + 1 == HOME_ITEMS.len();
                    let style = if selected {
                        s_sel()
                    } else if is_quit {
                        s_quit()
                    } else {
                        s_text()
                    };
                    let prefix = if selected { "> " } else { "  " };
                    ListItem::new(Span::styled(format!("{prefix}{label}"), style))
                })
                .collect();
            let list = List::new(items).block(block_main("Main menu — gaming setup"));
            f.render_widget(list, chunks[1]);
        }
        Screen::Apps => {
            let db = AppsDb::load(&app.paths).ok();
            // Whether the cursor row is a missing-prefix app (drives highlight_style color).
            let sel_missing = app
                .apps_state
                .selected()
                .and_then(|i| app.app_ids.get(i))
                .and_then(|id| db.as_ref().and_then(|d| d.apps.get(id)))
                .is_some_and(|e| !app.paths.prefix_path(&e.prefix).is_dir());
            let items: Vec<ListItem> = app
                .app_ids
                .iter()
                .map(|id| {
                    let (text, missing) = db
                        .as_ref()
                        .and_then(|d| d.apps.get(id))
                        .map(|e| {
                            // Missing = resolved path is not a directory (empty ensure_prefix dirs count as present).
                            let missing = !app.paths.prefix_path(&e.prefix).is_dir();
                            let loc = match e.locale.as_deref() {
                                Some(l) => format!("loc={l}"),
                                None => "loc=global".into(),
                            };
                            let gfx = match e.graphics.as_deref() {
                                Some(g) => format!("gfx={g}"),
                                None => "gfx=global".into(),
                            };
                            let prt = match e.proton.as_deref() {
                                Some(p) => format!("proton={p}"),
                                None => "proton=global".into(),
                            };
                            let pfx_mark = if missing {
                                format!("pfx={}  [NO PREFIX]", e.prefix)
                            } else {
                                format!("pfx={}", e.prefix)
                            };
                            let text = format!(
                                "{id}  —  {}  {pfx_mark}  {prt}  {loc}  {gfx}  {}",
                                e.name,
                                e.exe.display()
                            );
                            (text, missing)
                        })
                        .unwrap_or_else(|| (id.clone(), false));
                    // Unselected missing rows stay red; selected style comes from highlight_style.
                    if missing {
                        ListItem::new(Span::styled(text, s_err()))
                    } else {
                        ListItem::new(text)
                    }
                })
                .collect();
            let list = List::new(items)
                .block(block_main(
                    "Apps  [Enter/l]  [o/e]  [d]  [r]  [red = NO PREFIX]  [Esc]",
                ))
                .highlight_style(if sel_missing { s_err_sel() } else { s_sel() })
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.apps_state);
        }
        Screen::AddApp => {
            ui_add_app(f, app, chunks[1]);
        }
        Screen::Prefixes => {
            let items: Vec<ListItem> = app
                .prefix_ids
                .iter()
                .map(|id| {
                    let path = app.paths.prefix_path(id);
                    let is_default = id == &app.config.default_prefix;
                    let marker = if is_default { " [config default]" } else { "" };
                    let text = format!("{id}  →  {}{marker}", path.display());
                    if is_default {
                        ListItem::new(Span::styled(text, s_ok()))
                    } else {
                        ListItem::new(text)
                    }
                })
                .collect();
            let list = List::new(items)
                .block(block_main(
                    "Prefixes  [c create]  [d delete]  [s set default]  [Enter path]  [Esc]",
                ))
                .highlight_style(s_sel())
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.prefix_state);
        }
        Screen::Proton => {
            ui_proton_settings(f, app, chunks[1]);
        }
        Screen::Doctor => {
            let lines: Vec<Line> = app
                .doctor_lines
                .iter()
                .map(|line| {
                    let style = if line.contains("OK") || line.contains("umu") && line.contains("OK")
                    {
                        s_ok()
                    } else if line.contains("NONE")
                        || line.contains("missing")
                        || line.contains("unresolved")
                        || line.contains("not set")
                    {
                        s_warn()
                    } else if line.starts_with("  [") {
                        s_dim()
                    } else {
                        s_text()
                    };
                    Line::from(Span::styled(line.clone(), style))
                })
                .collect();
            let p = Paragraph::new(lines)
                .block(block_main("Doctor  [r refresh]  [Esc back]"))
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
        }
    }

    let status_style = if app.status.contains("failed")
        || app.status.contains("invalid")
        || app.status.contains("required")
        || app.status.contains("error")
        || app.status.contains("refusing")
        || app.status.contains("is used by apps")
    {
        s_warn()
    } else if app.status.contains("removed prefix")
        || app.status.contains("already gone")
    {
        s_ok()
    } else if app.status.contains("locale =")
        || app.status.contains("proton =")
        || app.status.contains("graphics =")
        || app.status.contains("sanitize_vulkan =")
        || app.status.contains("log_level =")
        || app.status.contains("gamescope =")
        || app.status.contains("env mode =")
        || app.status.contains("env `")
        || app.status.contains("global env")
        || app.status.contains("name →")
        || app.status.contains("default Proton")
        || app.status.contains("default proton")
        || app.status.contains("prefer =")
        || app.status.contains("registered")
        || app.status.contains("created")
        || app.status.contains("finished")
    {
        s_ok()
    } else {
        s_text()
    };
    let status = Paragraph::new(Span::styled(app.status.clone(), status_style)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(s_border_status())
            .title(Span::styled("status", s_accent())),
    );
    f.render_widget(status, chunks[2]);
}

/// Register app form: text fields with blink cursor, exe picker, prefix modes, proton list.
fn ui_add_app(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let resolved_id = add_resolved_id(app);
    let id_hint = if app.add_id.trim().is_empty() {
        format!("(auto: {resolved_id})")
    } else {
        String::new()
    };

    let cursor = |active: bool| -> Span<'static> {
        if active && app.cursor_blink_on {
            Span::styled("▌".to_string(), s_field_active())
        } else if active {
            Span::styled(" ".to_string(), s_dim())
        } else {
            Span::raw("")
        }
    };

    let text_row =
        |fld: AddField, label: &str, value: &str, extra: &str, typing: bool| -> Line<'static> {
            let active = app.add_field == fld;
            let mark = if active { ">" } else { " " };
            let label_s = if active {
                s_field_active()
            } else {
                s_field_idle()
            };
            // Focused values use a dark-gray island so yellow stays readable on light themes.
            let val_s = if active {
                s_focus_value()
            } else {
                s_text()
            };
            // Empty inactive fields show a placeholder; active empty fields rely on the blink caret.
            let display = if value.is_empty() && !active {
                "(empty)".to_string()
            } else if value.is_empty() && active && typing {
                String::new()
            } else {
                value.to_string()
            };
            let mut spans = vec![
                Span::styled(format!("{mark} {label}: "), label_s),
                Span::styled(display, val_s),
            ];
            if typing {
                spans.push(cursor(active));
            }
            if !extra.is_empty() {
                spans.push(Span::styled(format!(" {extra}"), s_dim()));
            }
            Line::from(spans)
        };

    // --- Prefix mode chips (default / auto / custom) ---
    let pfx_active = app.add_field == AddField::Prefix;
    let chip = |mode: AddPrefixMode, label: &str| -> Span<'static> {
        let on = app.add_prefix_mode == mode;
        let style = if on && pfx_active {
            s_sel()
        } else if on {
            s_active_item()
        } else if pfx_active {
            s_text()
        } else {
            s_dim()
        };
        let text = if on {
            format!("[{label}]")
        } else {
            format!(" {label} ")
        };
        Span::styled(text, style)
    };
    let pfx_preview = match app.add_prefix_mode {
        AddPrefixMode::Default => format!(
            "→ {}  ({})",
            app.config.default_prefix,
            app.paths.prefix_path(&app.config.default_prefix).display()
        ),
        AddPrefixMode::Auto => format!(
            "→ {resolved_id}  ({})",
            app.paths.prefix_path(&resolved_id).display()
        ),
        AddPrefixMode::Custom => {
            if app.add_prefix_custom.trim().is_empty() {
                "→ (type a relative id)".into()
            } else {
                let p = app.add_prefix_custom.trim();
                format!("→ {p}  ({})", app.paths.prefix_path(p).display())
            }
        }
    };
    let mut prefix_line = vec![
        Span::styled(
            if pfx_active { "> Prefix: " } else { "  Prefix: " }.to_string(),
            if pfx_active {
                s_field_active()
            } else {
                s_field_idle()
            },
        ),
        chip(AddPrefixMode::Default, "default"),
        Span::raw(" "),
        chip(AddPrefixMode::Auto, "auto"),
        Span::raw(" "),
        chip(AddPrefixMode::Custom, "custom"),
        Span::styled(format!("  {pfx_preview}"), s_dim()),
    ];
    if pfx_active && app.add_prefix_mode != AddPrefixMode::Custom {
        prefix_line.push(Span::styled("  (Enter cycle)", s_hint()));
    }

    let custom_row = if app.add_prefix_mode == AddPrefixMode::Custom {
        Some(text_row(
            AddField::Prefix,
            "  id   ",
            &app.add_prefix_custom,
            "(relative slug)",
            true,
        ))
    } else {
        None
    };

    // --- Exe picker row ---
    let exe_active = app.add_field == AddField::Exe;
    let exe_display = if app.add_exe.trim().is_empty() {
        "(Enter → built-in file picker)".to_string()
    } else {
        app.add_exe.clone()
    };
    let exe_line = Line::from(vec![
        Span::styled(
            if exe_active { "> Exe   : " } else { "  Exe   : " }.to_string(),
            if exe_active {
                s_field_active()
            } else {
                s_field_idle()
            },
        ),
        Span::styled(
            exe_display,
            if exe_active {
                s_focus_value()
            } else if app.add_exe.is_empty() {
                s_dim()
            } else {
                s_text()
            },
        ),
        if exe_active {
            Span::styled("  [Enter pick]", s_hint())
        } else {
            Span::raw("")
        },
    ]);

    // --- Proton list (expanded when field focused) ---
    let proton_active = app.add_field == AddField::Proton;
    let mut lines: Vec<Line> = vec![
        text_row(
            AddField::Name,
            "Name  ",
            &app.add_name,
            if app.add_name.is_empty() {
                "(optional → exe stem)"
            } else {
                ""
            },
            true,
        ),
        exe_line,
        text_row(AddField::Id, "Id    ", &app.add_id, &id_hint, true),
        Line::from(prefix_line),
    ];
    if let Some(row) = custom_row {
        lines.push(row);
    }

    lines.push(Span::styled(
        if proton_active {
            "> Proton:"
        } else {
            "  Proton:"
        }
        .to_string(),
        if proton_active {
            s_field_active()
        } else {
            s_field_idle()
        },
    )
    .into());

    let inherit_label = format!(
        "inherit global [{}]",
        app.config.proton.default
    );
    let n_picker = app_proton_picker_len(&app.proton_installs).max(1);
    let sel = app.add_proton_sel.min(n_picker - 1);

    if proton_active {
        // Row 0 = inherit; remaining rows are discovered installs.
        let inherit_on = sel == 0;
        lines.push(Line::from(Span::styled(
            format!(
                "{}{}",
                if inherit_on { "❯ " } else { "  " },
                inherit_label
            ),
            if inherit_on { s_sel() } else { s_hint() },
        )));
        if app.proton_installs.is_empty() {
            lines.push(Line::from(Span::styled(
                "    (none discovered — inherit until Proton settings [i] install)",
                s_warn(),
            )));
        } else {
            for (i, p) in app.proton_installs.iter().enumerate() {
                let on = sel == i + 1;
                let mark = if on { "❯ " } else { "  " };
                let style = if on { s_sel() } else { s_text() };
                lines.push(Line::from(Span::styled(
                    format!("{mark}[{}] {}  {}", p.source, p.name, p.path.display()),
                    style,
                )));
            }
        }
        lines.push(Line::from(Span::styled(
            "    ↑↓/jk/←→ cycle · Space next · Enter save form",
            s_hint(),
        )));
    } else {
        let text = if sel == 0 {
            format!("    {inherit_label}")
        } else if let Some(p) = app.proton_installs.get(sel - 1) {
            format!("    [{}] {}", p.source, p.name)
        } else {
            format!("    {inherit_label}")
        };
        lines.push(Line::from(Span::styled(text, s_text())));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Prefix: default = shared bottle · auto = prefixes/<app-id> · custom = type id",
        s_dim(),
    )));
    lines.push(Line::from(Span::styled(
        "Name empty → exe stem. Proton / locale / graphics / env inherit global unless overridden.",
        s_dim(),
    )));
    lines.push(Line::from(Span::styled(
        "Nav: Tab·↑↓·C-n/p  ·  Exe Enter=pick  ·  Prefix Enter=cycle  ·  Save=Enter on Name/Id/Proton  ·  Esc",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("Register app"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Proton settings: highlight the active default and let Enter pick another install.
fn ui_proton_settings(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let prefer = app.config.proton.prefer.as_str();
    let resolved =
        proton::resolve_proton_path(&app.paths, &app.config.proton, &app.config.proton.default).ok();

    let mut lines: Vec<Line> = Vec::new();

    // --- active summary (what will actually be used) ---
    lines.push(Line::from(vec![
        Span::styled("Using: ", s_dim()),
        Span::styled(app.config.proton.default.clone(), s_active_item()),
        Span::styled("   prefer=", s_dim()),
        Span::styled(
            prefer.to_string(),
            if prefer == "managed" || prefer == "steam" {
                s_active_item()
            } else {
                s_warn()
            },
        ),
    ]));
    match &resolved {
        Some(p) => lines.push(Line::from(vec![
            Span::styled("PROTONPATH: ", s_dim()),
            Span::styled(p.display().to_string(), s_ok()),
        ])),
        None => lines.push(Line::from(vec![
            Span::styled("PROTONPATH: ", s_dim()),
            Span::styled(
                "unresolved — install with [i] or pick a listed Proton",
                s_warn(),
            ),
        ])),
    }

    // prefer keys: color the active choice
    let managed_style = if prefer == "managed" {
        s_active_item()
    } else {
        s_hint()
    };
    let steam_style = if prefer == "steam" {
        s_active_item()
    } else {
        s_hint()
    };
    lines.push(Line::from(vec![
        Span::styled("keys: ", s_hint()),
        Span::styled("Enter", s_title()),
        Span::styled(" set default  ", s_hint()),
        Span::styled("[i]", s_hint()),
        Span::styled(" install  ", s_hint()),
        Span::styled("[m]", managed_style),
        Span::styled(" managed  ", managed_style),
        Span::styled("[t]", steam_style),
        Span::styled(" steam  ", steam_style),
        Span::styled("[d]", s_hint()),
        Span::styled(" dwproton  ", s_hint()),
        Span::styled("[r]", s_hint()),
        Span::styled(" refresh", s_hint()),
    ]));
    lines.push(Line::from(Span::styled(
        "★ = currently used   green = active   cyan cursor = selection",
        s_dim(),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Installed Proton:", s_title())));

    if app.proton_installs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none — press [i] to install dwproton, or install Proton via Steam)",
            s_warn(),
        )));
    } else {
        for (i, inst) in app.proton_installs.iter().enumerate() {
            let cursor = app.proton_sel == i;
            let active = app.is_active_proton(inst);
            let mark = match (active, cursor) {
                (true, _) => "★",
                (false, true) => ">",
                (false, false) => " ",
            };
            let style = match (active, cursor) {
                (true, true) => s_active_sel(),
                (true, false) => s_active_item(),
                (false, true) => s_sel_row(),
                (false, false) => s_text(),
            };
            // Show short path parent so managed vs steam is obvious beyond the source tag.
            let path_hint = inst
                .path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let label = format!(
                "{mark} [{src}] {name}  {hint}",
                src = inst.source,
                name = inst.name,
                hint = path_hint,
            );
            lines.push(Line::from(Span::styled(label, style)));
        }
    }

    let p = Paragraph::new(lines)
        .block(block_main(
            "Proton  —  ↑↓  Enter set default  Esc back",
        ))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_locale_picker(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let first_run = app.screen == Screen::LocaleSetup;
    let per_app = app.screen == Screen::AppLocale;

    let mut lines: Vec<Line> = Vec::new();

    if first_run {
        lines.push(Line::from(Span::styled(
            "Welcome to protoncli — choose the default launch locale.",
            s_title(),
        )));
    } else if per_app {
        let id = app.app_settings_id.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![
            Span::styled("Per-app locale for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]));
        lines.push(Line::from(Span::styled(
            "Overrides global only for this app. First option inherits global.",
            s_dim(),
        )));
    } else {
        let current = app
            .config
            .locale
            .as_deref()
            .and_then(locale::find)
            .map(|l| format!("{} ({})", l.id, l.posix))
            .unwrap_or_else(|| "(not set)".into());
        lines.push(Line::from(vec![
            Span::styled("Global default: ", s_dim()),
            Span::styled(current, s_ok()),
        ]));
        lines.push(Line::from(Span::styled(
            "Apps without a per-app locale inherit this setting.",
            s_dim(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Host does not need locale-gen; umu/Steam generates LANG/LC_* in-container.",
        s_hint(),
    )));
    lines.push(Line::from(""));

    if per_app {
        let n = locale::app_picker_len();
        for i in 0..n {
            let selected = i == app.app_locale_sel;
            let label = locale::app_picker_label(i, app.config.locale.as_deref());
            let mark = if selected { "> " } else { "  " };
            let style = if selected {
                s_sel()
            } else if i == 0 {
                s_hint()
            } else {
                s_text()
            };
            lines.push(Line::from(Span::styled(format!("{mark}{label}"), style)));
        }
    } else {
        for (i, loc) in locale::selectable().iter().enumerate() {
            let selected = i == app.locale_sel;
            let mark = if selected { "> " } else { "  " };
            let style = if selected {
                s_sel()
            } else {
                s_text()
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "{mark}{}  —  id={}  posix={}",
                    loc.label, loc.id, loc.posix
                ),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    let keys = if first_run {
        "↑↓ select · Enter save and continue · q quit"
    } else {
        "↑↓ select · Enter save · Esc back"
    };
    lines.push(Line::from(Span::styled(keys, s_hint())));

    let title = if first_run {
        "First-run locale"
    } else if per_app {
        "App locale  [Enter save]  [Esc back]"
    } else {
        "Global locale  [Enter save]  [Esc back]"
    };
    let block = if first_run {
        block_warn(title)
    } else {
        block_main(title)
    };
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_launch_settings(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let loc = app
        .config
        .locale
        .as_deref()
        .and_then(locale::find)
        .map(|l| format!("{} ({})", l.id, l.posix))
        .unwrap_or_else(|| "(not set)".into());
    let gfx = config::normalize_graphics(&app.config.launch.graphics);
    let sanitize = if app.config.launch.sanitize_vulkan {
        "ON"
    } else {
        "OFF"
    };
    let log_level = config::normalize_log_level(&app.config.launch.log_level);
    let log_hint = match log_level {
        "quiet" => "Hide umu/Proton/Wine stdout+stderr (use normal when debugging)",
        "verbose" => "Inherit streams + extra protoncli launch diagnostics",
        _ => "Show umu/Proton/Wine output on the terminal (default)",
    };
    let gs = app.config.launch.gamescope.status_label();
    let gs_hint = "Wrap launches: gamescope -f --force-windows-fullscreen -- <backend> (default off)";

    let rows: [(&str, String, &str); 5] = [
        (
            "Locale",
            loc,
            "LANG/LC_* for Japanese titles (umu generates in-container)",
        ),
        (
            "Graphics",
            gfx.into(),
            "dxvk = Vulkan (default) · wined3d = OpenGL fallback",
        ),
        (
            "Sanitize Vulkan",
            sanitize.into(),
            "Strip host VK_ICD_FILENAMES (Steam/Lutris-like; fixes 32-bit DXVK)",
        ),
        (
            "Log level",
            log_level.into(),
            log_hint,
        ),
        (
            "Gamescope",
            gs,
            gs_hint,
        ),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Defaults applied to every launch. Per-app overrides: Apps → o/e. Env: Home → Environment.",
            s_dim(),
        )),
        Line::from(""),
    ];
    for (i, (name, value, hint)) in rows.iter().enumerate() {
        let selected = i == app.launch_set_sel;
        let mark = if selected { "> " } else { "  " };
        let style = if selected {
            s_sel()
        } else {
            s_text()
        };
        lines.push(Line::from(Span::styled(
            format!("{mark}{name:<18} {value}"),
            style,
        )));
        if selected {
            lines.push(Line::from(Span::styled(format!("    {hint}"), s_hint())));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ · j/k · Ctrl+n/p  ·  Enter cycle/edit · Esc back",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("Launch settings"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_env_settings(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Global defaults for every launch (config [launch.env]).",
            s_dim(),
        )),
        Line::from(Span::styled(
            "Built-ins: DXVK_HUD=0 · STEAMOS=1 · UMU_USE_STEAM=1  (r restores them)",
            s_dim(),
        )),
        Line::from(Span::styled(
            "Per-app custom env is layered on top (same keys override). Apps → o/e → Env.",
            s_dim(),
        )),
        Line::from(""),
    ];

    let pairs: Vec<(String, String)> = app
        .config
        .launch
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if pairs.is_empty() {
        lines.push(Line::from(Span::styled(
            "(empty — press a or Enter to add)",
            s_warn(),
        )));
    } else {
        for (i, (k, v)) in pairs.iter().enumerate() {
            let selected = i == app.env_sel;
            let mark = if selected { "> " } else { "  " };
            let style = if selected {
                s_sel()
            } else {
                s_text()
            };
            lines.push(Line::from(Span::styled(
                format!("{mark}{k}={v}"),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓/jk/C-n C-p · Enter/e edit · a add · d delete · r reset · Esc home",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("Environment variables (global)"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_env_edit(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let scope = match app.env_scope {
        EnvScope::Global => "global defaults",
        EnvScope::App => "per-app custom (on top of defaults)",
    };
    let title = if app.env_edit_original_key.is_some() {
        "Edit environment variable"
    } else {
        "Add environment variable"
    };

    let key_active = app.env_edit_field == EnvEditField::Key;
    let val_active = app.env_edit_field == EnvEditField::Value;
    let row = |active: bool, label: &str, value: &str| -> Line<'static> {
        let mark = if active { ">" } else { " " };
        let label_s = if active {
            s_field_active()
        } else {
            s_field_idle()
        };
        let val_s = if active { s_focus_value() } else { s_text() };
        Line::from(vec![
            Span::styled(format!("{mark} {label}: "), label_s),
            Span::styled(value.to_string(), val_s),
        ])
    };

    let lines = vec![
        Line::from(Span::styled(format!("Scope: {scope}"), s_dim())),
        Line::from(""),
        row(key_active, "Key  ", &app.env_edit_key),
        row(val_active, "Value", &app.env_edit_value),
        Line::from(""),
        Line::from(Span::styled(
            "Field nav: Tab · ↑↓ · Ctrl+n/p · Ctrl+j/k  ·  Enter save · Esc cancel",
            s_hint(),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(block_main(title))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_app_name_edit(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let lines = vec![
        Line::from(vec![
            Span::styled("Display name for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Shown in the app list and .desktop Name= field. Id is unchanged.",
            s_dim(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("> Name: ", s_field_active()),
            Span::styled(app.app_settings_name.clone(), s_focus_value()),
        ]),
        Line::from(""),
        Line::from(Span::styled("Enter save · Esc cancel", s_hint())),
    ];
    let p = Paragraph::new(lines)
        .block(block_main("Edit app name"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_app_env(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let mode_label = if app.app_use_custom_env {
        "Custom"
    } else {
        "Default"
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Environment for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Default = only global [launch.env].  Custom = global defaults + extras below.",
            s_dim(),
        )),
        Line::from(Span::styled(
            "Same keys in Custom override the global defaults.",
            s_dim(),
        )),
        Line::from(""),
    ];

    // Mode row
    let mode_sel = app.app_env_sel == APP_ENV_MODE_ROW;
    lines.push(Line::from(Span::styled(
        format!(
            "{}Mode       {mode_label}",
            if mode_sel { "> " } else { "  " }
        ),
        if mode_sel {
            s_sel()
        } else {
            s_text()
        },
    )));
    if mode_sel {
        lines.push(Line::from(Span::styled(
            "    Enter toggles Default ↔ Custom",
            s_hint(),
        )));
    }

    lines.push(Line::from(""));
    if app.app_use_custom_env {
        lines.push(Line::from(Span::styled(
            "Custom extras (applied after defaults):",
            s_title(),
        )));
        let pairs: Vec<(String, String)> = AppsDb::load(&app.paths)
            .ok()
            .and_then(|db| {
                db.apps.get(id).map(|e| {
                    e.env
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
            })
            .unwrap_or_default();

        if pairs.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (none yet — press a to add)",
                s_dim(),
            )));
        } else {
            for (i, (k, v)) in pairs.iter().enumerate() {
                let row_idx = i + 1; // after mode row
                let selected = app.app_env_sel == row_idx;
                let mark = if selected { "> " } else { "  " };
                let style = if selected {
                    s_sel()
                } else {
                    s_text()
                };
                lines.push(Line::from(Span::styled(
                    format!("{mark}{k}={v}"),
                    style,
                )));
            }
        }

        // Preview effective overrides against globals for selected extra? skip — keep UI simple.
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Global defaults still applied:",
            s_dim(),
        )));
        for (k, v) in &app.config.launch.env {
            lines.push(Line::from(Span::styled(
                format!("  {k}={v}"),
                s_dim(),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Using global defaults only:",
            s_title(),
        )));
        if app.config.launch.env.is_empty() {
            lines.push(Line::from(Span::styled("  (global env empty)", s_dim())));
        } else {
            for (k, v) in &app.config.launch.env {
                lines.push(Line::from(Span::styled(
                    format!("  {k}={v}"),
                    s_dim(),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓/jk/C-n C-p · Enter toggle/edit · a add · d delete · Esc app settings",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("App environment"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_app_proton(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let n = app_proton_picker_len(&app.proton_installs).max(1);
    let sel = app.app_proton_sel.min(n - 1);

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Per-app Proton for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Overrides global only for this app. First option inherits config.proton.default.",
            s_dim(),
        )),
        Line::from(""),
    ];

    let inherit_on = sel == 0;
    lines.push(Line::from(Span::styled(
        format!(
            "{}Use global default  [{}]",
            if inherit_on { "> " } else { "  " },
            app.config.proton.default
        ),
        if inherit_on { s_sel() } else { s_hint() },
    )));

    if app.proton_installs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none discovered — install via Proton settings [i])",
            s_warn(),
        )));
    } else {
        for (i, inst) in app.proton_installs.iter().enumerate() {
            let on = sel == i + 1;
            let mark = if on { "> " } else { "  " };
            let style = if on { s_sel() } else { s_text() };
            lines.push(Line::from(Span::styled(
                format!(
                    "{mark}[{src}] {name}  {path}",
                    src = inst.source,
                    name = inst.name,
                    path = inst.path.display()
                ),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓/jk/C-n C-p select · Enter save · Esc cancel",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("App Proton  [Enter]  [Esc]"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_app_settings(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let name_disp = if app.app_settings_name.is_empty() {
        "(empty)".to_string()
    } else {
        app.app_settings_name.clone()
    };
    let exe_disp = if app.app_settings_exe.as_os_str().is_empty() {
        "(not set)".to_string()
    } else {
        app.app_settings_exe.display().to_string()
    };
    let proton_disp = match app_proton_picker_value(app.app_proton_sel, &app.proton_installs)
    {
        None => format!("inherit global [{}]", app.config.proton.default),
        Some(p) => p,
    };
    let loc = match locale::app_picker_value(app.app_locale_sel) {
        None => format!(
            "inherit global [{}]",
            app.config.locale.as_deref().unwrap_or("?")
        ),
        Some(l) => l.to_string(),
    };
    let gfx = match app_graphics_picker_value(app.app_graphics_sel) {
        None => format!(
            "inherit global [{}]",
            config::normalize_graphics(&app.config.launch.graphics)
        ),
        Some(g) => g.to_string(),
    };
    let gs = match app_gamescope_picker_value(app.app_gamescope_sel) {
        None => format!(
            "inherit global [{}]",
            app.config.launch.gamescope.status_label()
        ),
        Some(true) => "on".into(),
        Some(false) => "off".into(),
    };
    let env_disp = if app.app_use_custom_env {
        let n = AppsDb::load(&app.paths)
            .ok()
            .and_then(|db| db.apps.get(id).map(|e| e.env.len()))
            .unwrap_or(0);
        format!("custom (+{n} extras on defaults)")
    } else {
        "default (global [launch.env] only)".into()
    };

    let rows: [(&str, String); 7] = [
        ("Name", name_disp),
        ("Exe", exe_disp),
        ("Proton", proton_disp),
        ("Locale", loc),
        ("Graphics", gfx),
        ("Gamescope", gs),
        ("Env", env_disp),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Per-app settings for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Name: display label (id stays fixed). Exe: after installers, point at the real binary.",
            s_dim(),
        )),
        Line::from(Span::styled(
            "Env: Default = global only; Custom = defaults + extras (extras override).",
            s_dim(),
        )),
        Line::from(""),
    ];
    for (i, (name, value)) in rows.iter().enumerate() {
        let selected = i == app.app_set_sel;
        let mark = if selected { "> " } else { "  " };
        let style = if selected {
            s_sel()
        } else {
            s_text()
        };
        // Exe paths are long; keep the label short and let wrap show the rest.
        lines.push(Line::from(Span::styled(
            format!("{mark}{name:<10} {value}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Nav: ↑↓/jk/C-n C-p · Name · Exe · Proton · Locale · Graphics · Gamescope · Env",
        s_hint(),
    )));
    lines.push(Line::from(Span::styled("Esc back to Apps", s_hint())));

    // Show full graphics picker options for context.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Graphics options:", s_title())));
    for i in 0..3 {
        let mark = if app.app_graphics_sel == i {
            "* "
        } else {
            "  "
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{mark}{}",
                app_graphics_picker_label(i, &app.config.launch.graphics)
            ),
            if app.app_graphics_sel == i {
                s_ok()
            } else {
                s_dim()
            },
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Gamescope options:", s_title())));
    for i in 0..3 {
        let mark = if app.app_gamescope_sel == i {
            "* "
        } else {
            "  "
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{mark}{}",
                app_gamescope_picker_label(i, &app.config.launch.gamescope)
            ),
            if app.app_gamescope_sel == i {
                s_ok()
            } else {
                s_dim()
            },
        )));
    }

    let p = Paragraph::new(lines)
        .block(block_main("App settings  [Enter]  [Esc]"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_exe_browse_mode(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let prefix_path = AppsDb::load(&app.paths)
        .ok()
        .and_then(|db| db.apps.get(id).map(|e| app.paths.prefix_path(&e.prefix)))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(unknown prefix)".into());

    let rows: [(&str, String, &str); 2] = [
        (
            "Wineprefix",
            prefix_path,
            "Stay inside this bottle (starts at drive_c when present)",
        ),
        (
            "Filesystem",
            "host paths".into(),
            "Browse any directory; starts at current exe parent or $HOME",
        ),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Choose browse root for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Lists show directories and .exe only (other files hidden).",
            s_dim(),
        )),
        Line::from(""),
    ];
    for (i, (name, value, hint)) in rows.iter().enumerate() {
        let selected = i == app.exe_browse_mode_sel;
        let mark = if selected { "> " } else { "  " };
        let style = if selected {
            s_sel()
        } else {
            s_text()
        };
        lines.push(Line::from(Span::styled(
            format!("{mark}{name:<12} {value}"),
            style,
        )));
        if selected {
            lines.push(Line::from(Span::styled(format!("    {hint}"), s_hint())));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ select · Enter open browser · Esc back to App settings",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("Change exe — browse source"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_exe_browser(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    // Header + footer stay fixed; the middle List uses ListState so the
    // selection always stays in the visible viewport (scrolls when needed).
    let block = block_main("Pick exe");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // path + hint
            Constraint::Min(1),    // scrollable entries
            Constraint::Length(1), // keys
        ])
        .split(inner);

    let mode = if app.browse_floor.is_some() {
        "wineprefix"
    } else {
        "filesystem"
    };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(format!("[{mode}] "), s_accent()),
            Span::styled(app.browse_cwd.display().to_string(), s_ok()),
        ]),
        Line::from(Span::styled(
            "Only directories and .exe · Enter dir to open · Enter .exe to set",
            s_dim(),
        )),
    ]);
    f.render_widget(header, chunks[0]);

    let len = app.browse_list_len();
    if len == 0 {
        let empty = Paragraph::new(Span::styled(
            "  (empty — no dirs or .exe here)",
            s_warn(),
        ));
        f.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = (0..len)
            .filter_map(|i| {
                let (label, is_dir) = match app.browse_item_at(i) {
                    Some(BrowseItem::Parent) => ("../".to_string(), true),
                    Some(BrowseItem::Entry(e)) if e.is_dir => (format!("{}/", e.name), true),
                    Some(BrowseItem::Entry(e)) => (e.name.clone(), false),
                    None => return None,
                };
                // Dirs stay blue so navigation targets stand out from launchables.
                let style = if is_dir {
                    s_hint()
                } else {
                    s_text()
                };
                Some(ListItem::new(Span::styled(label, style)))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(s_sel())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut app.browse_state);
    }

    let keys = if app.browse_floor.is_none() {
        "↑↓ · Enter · Esc source · h/~ $HOME"
    } else {
        "↑↓ · Enter · Esc source (stays inside prefix)"
    };
    // Position indicator when the list can exceed one screenful.
    let footer = if len > 0 {
        let sel = app.browse_state.selected().unwrap_or(0) + 1;
        format!("{keys}  ·  {sel}/{len}")
    } else {
        keys.to_string()
    };
    f.render_widget(Paragraph::new(Span::styled(footer, s_hint())), chunks[2]);
}
