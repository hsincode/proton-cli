//! Minimal setup TUI (launched when proton-cli is run with no subcommand).
//!
//! Covers the settings a player typically needs before launching games:
//! app list / launch, register app, wine prefixes, Proton runtime, launch settings
//! (locale / graphics / Vulkan sanitize / log level), doctor.
//! On first run (`config.locale` unset) a locale picker is shown before the home menu.
//! Per-app locale / graphics / **exe** can be set from the Apps list (`o` key).
//! Exe change uses a filtered browser (dirs + `.exe` only) over the wineprefix or host FS.

use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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

use crate::apps::{new_entry, AppsDb};
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
const LAUNCH_SET_LEN: usize = 4;

/// Rows on the per-app settings screen.
const APP_SET_EXE: usize = 0;
const APP_SET_LOCALE: usize = 1;
const APP_SET_GRAPHICS: usize = 2;
const APP_SET_LEN: usize = 3;

/// Rows on the “where to browse for exe” chooser.
const EXE_MODE_PREFIX: usize = 0;
const EXE_MODE_FILESYSTEM: usize = 1;
const EXE_MODE_LEN: usize = 2;

// --- palette (subtle; readable on dark and light-ish terminals) ------------

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
    Style::default().fg(Color::DarkGray)
}
fn s_hint() -> Style {
    Style::default().fg(Color::Blue)
}
fn s_ok() -> Style {
    Style::default().fg(Color::Green)
}
fn s_warn() -> Style {
    Style::default().fg(Color::Yellow)
}
fn s_accent() -> Style {
    Style::default().fg(Color::Yellow)
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
    Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}
fn s_field_idle() -> Style {
    Style::default().fg(Color::Gray)
}
fn s_quit() -> Style {
    Style::default().fg(Color::Red)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// First-run only: must pick a locale before the rest of the TUI.
    LocaleSetup,
    /// Change global default locale (sub-picker from LaunchSettings).
    Locale,
    /// Global launch options: locale, graphics, Vulkan sanitize, log level.
    LaunchSettings,
    /// Per-app locale picker (inherit or override).
    AppLocale,
    /// Per-app launch options: exe + locale + graphics.
    AppSettings,
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
    add_prefix: String,
    add_proton: String,
    add_field: AddField,
    app_ids: Vec<String>,
    prefix_ids: Vec<String>,
    proton_names: Vec<String>,
    proton_sel: usize,
    /// Index for global locale picker (`selectable`).
    locale_sel: usize,
    /// Index for per-app locale picker (`app_picker_*`, 0 = inherit).
    app_locale_sel: usize,
    /// App id being edited on AppSettings / AppLocale / exe browser.
    app_settings_id: Option<String>,
    /// Current exe path for the app open in AppSettings (cached display + apply).
    app_settings_exe: PathBuf,
    /// Selected row on [`Screen::LaunchSettings`].
    launch_set_sel: usize,
    /// Selected row on [`Screen::AppSettings`].
    app_set_sel: usize,
    /// Per-app graphics picker index (0 = inherit, 1 = dxvk, 2 = wined3d).
    app_graphics_sel: usize,
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
    "Launch settings (locale / graphics / log)",
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

pub fn run(paths: Paths, config: Config) -> Result<()> {
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
            add_prefix: DEFAULT_PREFIX_ID.into(),
            add_proton: "dwproton".into(),
            add_field: AddField::Name,
            app_ids: Vec::new(),
            prefix_ids: Vec::new(),
            proton_names: Vec::new(),
            proton_sel: 0,
            locale_sel,
            app_locale_sel: 0,
            app_settings_id: None,
            app_settings_exe: PathBuf::new(),
            launch_set_sel: 0,
            app_set_sel: 0,
            app_graphics_sel: 0,
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
            "verbose" => "inherit streams + extra proton-cli diagnostics",
            _ => "show umu/Proton/Wine output (default)",
        };
        self.status = format!("log_level = {next} ({hint})");
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
        self.app_locale_sel = locale::app_picker_index(entry.locale.as_deref());
        self.app_graphics_sel = app_graphics_picker_index(entry.graphics.as_deref());
        self.app_settings_exe = entry.exe.clone();
        self.app_settings_id = Some(id.clone());
        self.app_set_sel = 0;
        self.status = format!("settings for `{id}`  ↑↓  Enter edit  Esc back");
        self.screen = Screen::AppSettings;
        Ok(())
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
        self.browse_cwd = browse::start_dir_for_filesystem(&self.app_settings_exe);
        self.reload_browse_entries()?;
        self.status =
            "filesystem browse  ↑↓  Enter open/select  Esc mode  (dirs + .exe only)".into();
        self.screen = Screen::ExeBrowser;
        Ok(())
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

        let installs = proton::list_installs(&self.paths).unwrap_or_default();
        self.proton_names = installs
            .iter()
            .map(|p| format!("[{}] {}  {}", p.source, p.name, p.path.display()))
            .collect();
        if self.proton_sel >= self.proton_names.len() {
            self.proton_sel = 0;
        }
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
        if report.protons.is_empty() {
            lines.push("Proton    : none".into());
        } else {
            for p in &report.protons {
                lines.push(format!("  [{}] {}", p.source, p.name));
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
                    handle_key(app, key.code)?;
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

fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    match app.screen {
        Screen::LocaleSetup | Screen::Locale | Screen::AppLocale => handle_locale_key(app, code)?,
        Screen::LaunchSettings => handle_launch_settings_key(app, code)?,
        Screen::AppSettings => handle_app_settings_key(app, code)?,
        Screen::ExeBrowseMode => handle_exe_browse_mode_key(app, code)?,
        Screen::ExeBrowser => handle_exe_browser_key(app, code)?,
        Screen::Home => match code {
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => {
                if app.home_idx > 0 {
                    app.home_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.home_idx + 1 < HOME_ITEMS.len() {
                    app.home_idx += 1;
                }
            }
            KeyCode::Enter => match app.home_idx {
                0 => {
                    app.refresh_lists()?;
                    app.screen = Screen::Apps;
                    app.status = "Enter/l launch · o settings · d remove · Esc back".into();
                }
                1 => {
                    app.add_name.clear();
                    app.add_exe.clear();
                    app.add_id.clear();
                    app.add_prefix = app.config.default_prefix.clone();
                    app.add_proton = app.config.proton.default.clone();
                    app.add_field = AddField::Name;
                    app.screen = Screen::AddApp;
                    app.status = "Tab fields · Enter save · Esc cancel".into();
                }
                2 => {
                    app.refresh_lists()?;
                    app.screen = Screen::Prefixes;
                    app.status = "c create · s set default · Esc back".into();
                }
                3 => {
                    app.refresh_lists()?;
                    app.screen = Screen::Proton;
                    app.status = "i install · m/t prefer · Esc back".into();
                }
                4 => {
                    app.launch_set_sel = 0;
                    app.status =
                        "Launch settings  ↑↓  Enter edit  Esc back (locale/graphics/log)"
                            .into();
                    app.screen = Screen::LaunchSettings;
                }
                5 => {
                    app.refresh_doctor();
                    app.screen = Screen::Doctor;
                    app.status = "r refresh · Esc back".into();
                }
                6 => app.should_quit = true,
                _ => {}
            },
            _ => {}
        },
        Screen::Apps => match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Home;
                app.status = "↑↓ move  Enter select  Esc back  q quit".into();
            }
            KeyCode::Up | KeyCode::Char('k') => select_prev(&mut app.apps_state, app.app_ids.len()),
            KeyCode::Down | KeyCode::Char('j') => {
                select_next(&mut app.apps_state, app.app_ids.len())
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
            KeyCode::Char('o') => app.open_app_settings()?,
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
        },
        Screen::AddApp => match code {
            KeyCode::Esc => {
                app.screen = Screen::Home;
                app.status = "↑↓ move  Enter select  Esc back  q quit".into();
            }
            KeyCode::Tab | KeyCode::Down => {
                app.add_field = match app.add_field {
                    AddField::Name => AddField::Exe,
                    AddField::Exe => AddField::Id,
                    AddField::Id => AddField::Prefix,
                    AddField::Prefix => AddField::Proton,
                    AddField::Proton => AddField::Name,
                };
            }
            KeyCode::BackTab | KeyCode::Up => {
                app.add_field = match app.add_field {
                    AddField::Name => AddField::Proton,
                    AddField::Exe => AddField::Name,
                    AddField::Id => AddField::Exe,
                    AddField::Prefix => AddField::Id,
                    AddField::Proton => AddField::Prefix,
                };
            }
            KeyCode::Char(c) if !c.is_control() => {
                field_mut(app).push(c);
            }
            KeyCode::Backspace => {
                field_mut(app).pop();
            }
            KeyCode::Enter => save_new_app(app)?,
            _ => {}
        },
        Screen::Prefixes => match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Home;
                app.status = "↑↓ move  Enter select  Esc back  q quit".into();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                select_prev(&mut app.prefix_state, app.prefix_ids.len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                select_next(&mut app.prefix_state, app.prefix_ids.len())
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
        },
        Screen::Proton => match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Home;
                app.status = "↑↓ move  Enter select  Esc back  q quit".into();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !app.proton_names.is_empty() {
                    app.proton_sel = if app.proton_sel == 0 {
                        app.proton_names.len() - 1
                    } else {
                        app.proton_sel - 1
                    };
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !app.proton_names.is_empty() {
                    app.proton_sel = (app.proton_sel + 1) % app.proton_names.len();
                }
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
                app.status = "prefer = managed".into();
            }
            KeyCode::Char('t') => {
                app.config.proton.prefer = "steam".into();
                app.config.save(&app.paths)?;
                app.status = "prefer = steam".into();
            }
            KeyCode::Char('d') => {
                app.config.proton.default = "dwproton".into();
                app.config.save(&app.paths)?;
                app.status = "default proton = dwproton".into();
            }
            KeyCode::Char('r') => {
                app.refresh_lists()?;
                app.status = "proton list refreshed".into();
            }
            _ => {}
        },
        Screen::Doctor => match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Home;
                app.status = "↑↓ move  Enter select  Esc back  q quit".into();
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

fn handle_locale_key(app: &mut App, code: KeyCode) -> Result<()> {
    let first_run = app.screen == Screen::LocaleSetup;
    let per_app = app.screen == Screen::AppLocale;
    let n = if per_app {
        locale::app_picker_len()
    } else {
        locale::selectable().len()
    };

    match code {
        KeyCode::Esc if !first_run => {
            if per_app {
                app.screen = Screen::AppSettings;
                app.status = "↑↓ row  Enter edit  Esc back".into();
            } else {
                // Global locale picker is nested under Launch settings.
                app.screen = Screen::LaunchSettings;
                app.status = "Launch settings  ↑↓  Enter edit  Esc back".into();
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
        KeyCode::Up | KeyCode::Char('k') => {
            if n > 0 {
                let sel = if per_app {
                    &mut app.app_locale_sel
                } else {
                    &mut app.locale_sel
                };
                *sel = if *sel == 0 { n - 1 } else { *sel - 1 };
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if n > 0 {
                let sel = if per_app {
                    &mut app.app_locale_sel
                } else {
                    &mut app.locale_sel
                };
                *sel = (*sel + 1) % n;
            }
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

fn handle_launch_settings_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::Home;
            app.status = "↑↓ move  Enter select  Esc back  q quit".into();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.launch_set_sel = if app.launch_set_sel == 0 {
                LAUNCH_SET_LEN - 1
            } else {
                app.launch_set_sel - 1
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.launch_set_sel = (app.launch_set_sel + 1) % LAUNCH_SET_LEN;
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.launch_set_sel {
            LAUNCH_SET_LOCALE => {
                app.locale_sel = locale::selectable_index(app.config.locale.as_deref());
                app.status = "Global default locale  ↑↓  Enter save  Esc back".into();
                app.screen = Screen::Locale;
            }
            LAUNCH_SET_GRAPHICS => app.cycle_global_graphics()?,
            LAUNCH_SET_SANITIZE => app.toggle_sanitize_vulkan()?,
            LAUNCH_SET_LOG_LEVEL => app.cycle_log_level()?,
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_app_settings_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.app_settings_id = None;
            app.screen = Screen::Apps;
            app.status = "Enter/l launch · o settings · d remove · Esc back".into();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.app_set_sel = if app.app_set_sel == 0 {
                APP_SET_LEN - 1
            } else {
                app.app_set_sel - 1
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.app_set_sel = (app.app_set_sel + 1) % APP_SET_LEN;
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.app_set_sel {
            APP_SET_EXE => app.open_exe_browse_mode(),
            APP_SET_LOCALE => {
                let id = app.app_settings_id.as_deref().unwrap_or("?");
                app.status = format!("locale for `{id}`  ↑↓  Enter save  Esc back");
                app.screen = Screen::AppLocale;
            }
            APP_SET_GRAPHICS => {
                // Cycle inherit → dxvk → wined3d → inherit.
                app.app_graphics_sel = (app.app_graphics_sel + 1) % 3;
                app.apply_app_graphics()?;
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_exe_browse_mode_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            let id = app.app_settings_id.as_deref().unwrap_or("?");
            app.status = format!("settings for `{id}`  ↑↓  Enter edit  Esc back");
            app.screen = Screen::AppSettings;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.exe_browse_mode_sel = if app.exe_browse_mode_sel == 0 {
                EXE_MODE_LEN - 1
            } else {
                app.exe_browse_mode_sel - 1
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.exe_browse_mode_sel = (app.exe_browse_mode_sel + 1) % EXE_MODE_LEN;
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.exe_browse_mode_sel {
            EXE_MODE_PREFIX => app.start_browse_prefix()?,
            EXE_MODE_FILESYSTEM => app.start_browse_filesystem()?,
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_exe_browser_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.open_exe_browse_mode();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let len = app.browse_list_len();
            select_prev(&mut app.browse_state, len);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.browse_list_len();
            select_next(&mut app.browse_state, len);
        }
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
                BrowseAction::PickExe(path) => {
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
            }
        }
        // Quick jump: home directory (filesystem mode only — prefix floor ignores this).
        KeyCode::Char('~') | KeyCode::Char('h') if app.browse_floor.is_none() => {
            if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
                app.browse_cwd = home;
                app.browse_state.select(Some(0));
                app.reload_browse_entries()?;
                app.status = "jumped to $HOME".into();
            }
        }
        _ => {}
    }
    Ok(())
}

enum BrowseAction {
    Up,
    EnterDir(PathBuf),
    PickExe(PathBuf),
}

fn save_new_app(app: &mut App) -> Result<()> {
    let name = app.add_name.trim().to_string();
    let exe = PathBuf::from(app.add_exe.trim());
    if name.is_empty() || app.add_exe.trim().is_empty() {
        app.status = "name and exe are required".into();
        return Ok(());
    }
    let id = if app.add_id.trim().is_empty() {
        slugify(&name)
    } else {
        app.add_id.trim().to_string()
    };
    if !is_valid_app_id(&id) {
        app.status = format!("invalid id `{id}`");
        return Ok(());
    }
    let prefix = if app.add_prefix.trim().is_empty() {
        app.config.default_prefix.clone()
    } else {
        app.add_prefix.trim().to_string()
    };
    if !PathBuf::from(&prefix).is_absolute()
        && prefix != DEFAULT_PREFIX_ID
        && !is_valid_app_id(&prefix)
    {
        app.status = format!("invalid prefix id `{prefix}`");
        return Ok(());
    }
    let proton = if app.add_proton.trim().is_empty() {
        "dwproton".into()
    } else {
        app.add_proton.trim().to_string()
    };
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

fn field_mut(app: &mut App) -> &mut String {
    match app.add_field {
        AddField::Name => &mut app.add_name,
        AddField::Exe => &mut app.add_exe,
        AddField::Id => &mut app.add_id,
        AddField::Prefix => &mut app.add_prefix,
        AddField::Proton => &mut app.add_proton,
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
        Screen::AppLocale => "Locale (app)",
        Screen::AppSettings => "App settings",
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
        Span::styled(" proton-cli ", s_title()),
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
        Screen::LaunchSettings => ui_launch_settings(f, app, chunks[1]),
        Screen::AppSettings => ui_app_settings(f, app, chunks[1]),
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
                        Style::default().fg(Color::White)
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
            let items: Vec<ListItem> = app
                .app_ids
                .iter()
                .map(|id| {
                    let detail = db
                        .as_ref()
                        .and_then(|d| d.apps.get(id))
                        .map(|e| {
                            let loc = match e.locale.as_deref() {
                                Some(l) => format!("loc={l}"),
                                None => "loc=global".into(),
                            };
                            let gfx = match e.graphics.as_deref() {
                                Some(g) => format!("gfx={g}"),
                                None => "gfx=global".into(),
                            };
                            format!(
                                "{id}  —  {}  pfx={}  {loc}  {gfx}  {}",
                                e.name,
                                e.prefix,
                                e.exe.display()
                            )
                        })
                        .unwrap_or_else(|| id.clone());
                    ListItem::new(detail)
                })
                .collect();
            let list = List::new(items)
                .block(block_main(
                    "Apps  [Enter/l launch]  [o settings]  [d remove]  [r refresh]  [Esc]",
                ))
                .highlight_style(s_sel())
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.apps_state);
        }
        Screen::AddApp => {
            let id_hint = if app.add_id.is_empty() {
                format!("(auto: {})", slugify(&app.add_name))
            } else {
                String::new()
            };
            let row = |fld: AddField, label: &str, value: &str, extra: &str| -> Line<'static> {
                let active = app.add_field == fld;
                let mark = if active { ">" } else { " " };
                let label_s = if active { s_field_active() } else { s_field_idle() };
                let val_s = if active {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(vec![
                    Span::styled(format!("{mark} {label}: "), label_s),
                    Span::styled(value.to_string(), val_s),
                    Span::styled(format!(" {extra}"), s_dim()),
                ])
            };
            let lines = vec![
                row(AddField::Name, "Name  ", &app.add_name, ""),
                row(AddField::Exe, "Exe   ", &app.add_exe, ""),
                row(AddField::Id, "Id    ", &app.add_id, &id_hint),
                row(AddField::Prefix, "Prefix", &app.add_prefix, ""),
                row(AddField::Proton, "Proton", &app.add_proton, ""),
                Line::from(""),
                Line::from(Span::styled(
                    "Prefix: `default` → ~/.protoncli/default; other id → prefixes/<id>",
                    s_dim(),
                )),
                Line::from(Span::styled(
                    "Locale/graphics inherit global (Apps → o). Tab · Enter save · Esc",
                    s_hint(),
                )),
            ];
            let p = Paragraph::new(lines)
                .block(block_main("Register app"))
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
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
                    "Prefixes  [c create pfx-N]  [s set default]  [Enter path]  [Esc]",
                ))
                .highlight_style(s_sel())
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.prefix_state);
        }
        Screen::Proton => {
            let mut lines: Vec<Line> = vec![
                Line::from(vec![
                    Span::styled("config: ", s_dim()),
                    Span::styled(
                        format!(
                            "default={}  prefer={}",
                            app.config.proton.default, app.config.proton.prefer
                        ),
                        s_accent(),
                    ),
                ]),
                Line::from(Span::styled(
                    "keys: [i] install dwproton  [m] managed  [t] steam  [d] default=dwproton  [r]",
                    s_hint(),
                )),
                Line::from(""),
                Line::from(Span::styled("Installed:", s_title())),
            ];
            if app.proton_names.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (none — press i to install)",
                    s_warn(),
                )));
            } else {
                for (i, name) in app.proton_names.iter().enumerate() {
                    let sel = app.proton_sel == i;
                    let style = if sel { s_sel_row() } else { Style::default() };
                    lines.push(Line::from(Span::styled(
                        format!("{} {name}", if sel { ">" } else { " " }),
                        style,
                    )));
                }
            }
            let p = Paragraph::new(lines)
                .block(block_main("Proton"))
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
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
                        Style::default().fg(Color::White)
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
    {
        s_warn()
    } else if app.status.contains("locale =")
        || app.status.contains("graphics =")
        || app.status.contains("sanitize_vulkan =")
        || app.status.contains("log_level =")
        || app.status.contains("registered")
        || app.status.contains("created")
        || app.status.contains("finished")
    {
        s_ok()
    } else {
        Style::default().fg(Color::White)
    };
    let status = Paragraph::new(Span::styled(app.status.clone(), status_style)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(s_border_status())
            .title(Span::styled("status", s_accent())),
    );
    f.render_widget(status, chunks[2]);
}

fn ui_locale_picker(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let first_run = app.screen == Screen::LocaleSetup;
    let per_app = app.screen == Screen::AppLocale;

    let mut lines: Vec<Line> = Vec::new();

    if first_run {
        lines.push(Line::from(Span::styled(
            "Welcome to proton-cli — choose the default launch locale.",
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
                Style::default().fg(Color::White)
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
                Style::default().fg(Color::White)
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
        "verbose" => "Inherit streams + extra proton-cli launch diagnostics",
        _ => "Show umu/Proton/Wine output on the terminal (default)",
    };

    let rows: [(&str, String, &str); 4] = [
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
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Defaults applied to every launch. Per-app overrides: Apps → o.",
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
            Style::default().fg(Color::White)
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
        "↑↓ select · Enter cycle/edit · Esc back",
        s_hint(),
    )));

    let p = Paragraph::new(lines)
        .block(block_main("Launch settings"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn ui_app_settings(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let id = app.app_settings_id.as_deref().unwrap_or("?");
    let exe_disp = if app.app_settings_exe.as_os_str().is_empty() {
        "(not set)".to_string()
    } else {
        app.app_settings_exe.display().to_string()
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

    let rows: [(&str, String); 3] = [
        ("Exe", exe_disp),
        ("Locale", loc),
        ("Graphics", gfx),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Per-app settings for ", s_dim()),
            Span::styled(id.to_string(), s_accent()),
        ]),
        Line::from(Span::styled(
            "Exe: after installers, point at the real game binary. Locale/graphics inherit global when unset.",
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
            Style::default().fg(Color::White)
        };
        // Exe paths are long; keep the label short and let wrap show the rest.
        lines.push(Line::from(Span::styled(
            format!("{mark}{name:<10} {value}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Exe: Enter → browse (prefix or host) · Locale: picker · Graphics: cycle",
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
            Style::default().fg(Color::White)
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

fn ui_exe_browser(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let mode = if app.browse_floor.is_some() {
        "wineprefix"
    } else {
        "filesystem"
    };
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(format!("[{mode}] "), s_accent()),
            Span::styled(app.browse_cwd.display().to_string(), s_ok()),
        ]),
        Line::from(Span::styled(
            "Only directories and .exe · Enter dir to open · Enter .exe to set",
            s_dim(),
        )),
        Line::from(""),
    ];

    let len = app.browse_list_len();
    if len == 0 {
        lines.push(Line::from(Span::styled(
            "  (empty — no dirs or .exe here)",
            s_warn(),
        )));
    } else {
        let sel = app.browse_state.selected();
        for i in 0..len {
            let selected = sel == Some(i);
            let mark = if selected { "> " } else { "  " };
            let style = if selected {
                s_sel()
            } else {
                Style::default().fg(Color::White)
            };
            let label = match app.browse_item_at(i) {
                Some(BrowseItem::Parent) => "../".to_string(),
                Some(BrowseItem::Entry(e)) if e.is_dir => format!("{}/", e.name),
                Some(BrowseItem::Entry(e)) => e.name.clone(),
                None => continue,
            };
            let row_style = if selected {
                style
            } else if matches!(app.browse_item_at(i), Some(BrowseItem::Entry(e)) if e.is_dir)
                || matches!(app.browse_item_at(i), Some(BrowseItem::Parent))
            {
                s_hint()
            } else {
                style
            };
            lines.push(Line::from(Span::styled(
                format!("{mark}{label}"),
                row_style,
            )));
        }
    }

    lines.push(Line::from(""));
    let keys = if app.browse_floor.is_none() {
        "↑↓ · Enter · Esc source · h/~ $HOME"
    } else {
        "↑↓ · Enter · Esc source (stays inside prefix)"
    };
    lines.push(Line::from(Span::styled(keys, s_hint())));

    let p = Paragraph::new(lines)
        .block(block_main("Pick exe"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
