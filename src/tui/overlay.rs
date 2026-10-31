//! Modal layers drawn over the active tab (pickers, forms, dialogs, browser).
//!
//! Overlays live on a stack: the top one receives keys. A form can open
//! another overlay (e.g. Add app → file browser) and get the result back when
//! the upper one closes.

use std::path::{Path, PathBuf};

use crate::browse::{self, BrowseEntry};
use crate::config;
use crate::error::Result;
use crate::locale;
use crate::proton::ProtonInstall;

use super::app::{Action, App};
use super::text::TextInput;

pub enum Overlay {
    Picker(Picker),
    Input(InputDialog),
    Confirm(Confirm),
    AddApp(AddForm),
    AppEnv(AppEnv),
    EnvEdit(EnvForm),
    Browser(Browser),
    Palette(Palette),
    Help,
}

// --- picker -----------------------------------------------------------------

pub struct PickItem {
    pub label: String,
    pub hint: String,
}

impl PickItem {
    pub fn new(label: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
        }
    }
}

#[derive(Clone)]
pub enum PickTarget {
    /// First launch: a locale must be chosen before anything else.
    FirstRunLocale,
    GlobalLocale,
    AppProton(String),
    AppLocale(String),
    AppGraphics(String),
    AppGamescope(String),
    /// Where to browse for a new exe of this app (wineprefix / host FS).
    ExeSource(String),
}

pub struct Picker {
    pub title: String,
    pub items: Vec<PickItem>,
    pub sel: usize,
    pub target: PickTarget,
}

/// Per-app Proton choices: inherit (`None`) followed by every discovered install.
pub fn proton_options(installs: &[ProtonInstall]) -> Vec<Option<String>> {
    std::iter::once(None)
        .chain(installs.iter().map(|p| Some(p.name.clone())))
        .collect()
}

pub fn locale_options() -> Vec<Option<&'static str>> {
    std::iter::once(None)
        .chain(locale::selectable().iter().map(|l| Some(l.id)))
        .collect()
}

pub const GRAPHICS_OPTIONS: [Option<&str>; 3] = [None, Some("dxvk"), Some("wined3d")];
pub const GAMESCOPE_OPTIONS: [Option<bool>; 3] = [None, Some(true), Some(false)];

pub fn graphics_hint(id: &str) -> &'static str {
    match config::normalize_graphics(id) {
        "wined3d" => "Direct3D → OpenGL (fallback when DXVK fails)",
        _ => "Direct3D → Vulkan (default, like Steam)",
    }
}

impl Picker {
    pub fn first_run_locale() -> Self {
        let mut p = Self::global_locale(None);
        p.title = "Welcome — choose a launch locale".into();
        p.target = PickTarget::FirstRunLocale;
        p
    }

    pub fn global_locale(current: Option<&str>) -> Self {
        let items = locale::selectable()
            .iter()
            .map(|l| PickItem::new(l.label, l.posix))
            .collect();
        Self {
            title: "Default locale".into(),
            items,
            sel: locale::selectable_index(current),
            target: PickTarget::GlobalLocale,
        }
    }

    pub fn app_proton(app: &App, id: &str) -> Self {
        let current = app.db.apps.get(id).and_then(|e| e.proton.clone());
        let opts = proton_options(&app.protons);
        let items = opts
            .iter()
            .map(|o| match o {
                None => PickItem::new(
                    "Use global default",
                    app.describe_proton(&app.config.proton.default),
                ),
                Some(name) => {
                    let src = app
                        .protons
                        .iter()
                        .find(|p| &p.name == name)
                        .map(|p| p.source)
                        .unwrap_or("");
                    PickItem::new(name.clone(), src)
                }
            })
            .collect();
        Self {
            title: "Proton".into(),
            items,
            sel: opts.iter().position(|o| *o == current).unwrap_or(0),
            target: PickTarget::AppProton(id.to_string()),
        }
    }

    pub fn app_locale(app: &App, id: &str) -> Self {
        let current = app.db.apps.get(id).and_then(|e| e.locale.clone());
        let opts = locale_options();
        let global = app
            .config
            .locale
            .as_deref()
            .and_then(locale::find)
            .map(|l| l.posix)
            .unwrap_or("not set");
        let items = opts
            .iter()
            .map(|o| match o.and_then(locale::find) {
                None => PickItem::new("Use global default", global),
                Some(l) => PickItem::new(l.label, l.posix),
            })
            .collect();
        Self {
            title: "Locale".into(),
            items,
            sel: opts
                .iter()
                .position(|o| o.map(str::to_string) == current)
                .unwrap_or(0),
            target: PickTarget::AppLocale(id.to_string()),
        }
    }

    pub fn app_graphics(app: &App, id: &str) -> Self {
        let current = app.db.apps.get(id).and_then(|e| e.graphics.clone());
        let global = config::normalize_graphics(&app.config.launch.graphics);
        let items = GRAPHICS_OPTIONS
            .iter()
            .map(|o| match o {
                None => PickItem::new("Use global default", global),
                Some(g) => PickItem::new(*g, graphics_hint(g)),
            })
            .collect();
        Self {
            title: "Graphics".into(),
            items,
            sel: match current.as_deref() {
                None => 0,
                Some("wined3d") => 2,
                Some(_) => 1,
            },
            target: PickTarget::AppGraphics(id.to_string()),
        }
    }

    pub fn app_gamescope(app: &App, id: &str) -> Self {
        let current = app.db.apps.get(id).and_then(|e| e.gamescope);
        let items = vec![
            PickItem::new(
                "Use global default",
                app.config.launch.gamescope.status_label(),
            ),
            PickItem::new("on", "wrap the game in gamescope"),
            PickItem::new("off", "plain umu / Proton"),
        ];
        Self {
            title: "Gamescope".into(),
            items,
            sel: GAMESCOPE_OPTIONS
                .iter()
                .position(|o| *o == current)
                .unwrap_or(0),
            target: PickTarget::AppGamescope(id.to_string()),
        }
    }

    pub fn exe_source(app: &App, id: &str) -> Self {
        let prefix = app
            .db
            .apps
            .get(id)
            .map(|e| app.paths.prefix_path(&e.prefix).display().to_string())
            .unwrap_or_default();
        Self {
            title: "Browse for exe in".into(),
            items: vec![
                PickItem::new("Wineprefix", prefix),
                PickItem::new("Filesystem", "any host directory"),
            ],
            sel: 0,
            target: PickTarget::ExeSource(id.to_string()),
        }
    }
}

// --- text input dialog ------------------------------------------------------

pub enum InputTarget {
    AppName(String),
    NewPrefix,
}

pub struct InputDialog {
    pub title: String,
    pub label: String,
    pub input: TextInput,
    pub target: InputTarget,
    pub error: Option<String>,
}

// --- confirm ----------------------------------------------------------------

pub enum ConfirmAction {
    RemoveApp(String),
    DeletePrefix(String),
    ResetGlobalEnv,
}

pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    pub action: ConfirmAction,
}

// --- register app form ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixMode {
    /// `config.default_prefix` (shared bottle).
    Default,
    /// Own bottle named after the app id.
    Auto,
    /// Typed prefix id.
    Custom,
}

impl PrefixMode {
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Auto,
            Self::Auto => Self::Custom,
            Self::Custom => Self::Default,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Default => Self::Custom,
            Self::Auto => Self::Default,
            Self::Custom => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddField {
    Exe,
    Name,
    Id,
    Prefix,
    Proton,
    Save,
}

impl AddField {
    pub const ALL: [AddField; 6] = [
        AddField::Exe,
        AddField::Name,
        AddField::Id,
        AddField::Prefix,
        AddField::Proton,
        AddField::Save,
    ];

    pub fn step(self, dir: i32) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0) as i32;
        let n = Self::ALL.len() as i32;
        Self::ALL[(i + dir).rem_euclid(n) as usize]
    }
}

pub struct AddForm {
    pub exe: TextInput,
    pub name: TextInput,
    pub id: TextInput,
    pub prefix_mode: PrefixMode,
    pub prefix_custom: TextInput,
    /// Index into [`proton_options`] (0 = inherit global).
    pub proton_sel: usize,
    pub field: AddField,
    pub error: Option<String>,
}

impl AddForm {
    pub fn new() -> Self {
        Self {
            exe: TextInput::default(),
            name: TextInput::default(),
            id: TextInput::default(),
            prefix_mode: PrefixMode::Default,
            prefix_custom: TextInput::default(),
            proton_sel: 0,
            field: AddField::Exe,
            error: None,
        }
    }

    /// Name that will be saved (typed, else the exe stem).
    pub fn resolved_name(&self) -> String {
        if self.name.is_empty() {
            crate::paths::name_from_exe(Path::new(self.exe.trimmed()))
        } else {
            self.name.trimmed().to_string()
        }
    }

    /// Id that will be saved (typed, else a slug of the name).
    pub fn resolved_id(&self) -> String {
        if self.id.is_empty() {
            crate::paths::slugify(&self.resolved_name())
        } else {
            self.id.trimmed().to_string()
        }
    }

    pub fn resolved_prefix(&self, default_prefix: &str) -> Option<String> {
        match self.prefix_mode {
            PrefixMode::Default => Some(default_prefix.to_string()),
            PrefixMode::Auto => Some(self.resolved_id()),
            PrefixMode::Custom => {
                (!self.prefix_custom.is_empty()).then(|| self.prefix_custom.trimmed().to_string())
            }
        }
    }

    /// Focused text buffer, if the focused field accepts typing.
    pub fn text_mut(&mut self) -> Option<&mut TextInput> {
        match self.field {
            AddField::Exe => Some(&mut self.exe),
            AddField::Name => Some(&mut self.name),
            AddField::Id => Some(&mut self.id),
            AddField::Prefix if self.prefix_mode == PrefixMode::Custom => {
                Some(&mut self.prefix_custom)
            }
            _ => None,
        }
    }
}

// --- env editors ------------------------------------------------------------

/// Per-app env overlay: row 0 toggles default/custom, rows 1.. are extra pairs.
pub struct AppEnv {
    pub id: String,
    pub sel: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub enum EnvScope {
    Global,
    App(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvField {
    Key,
    Value,
}

pub struct EnvForm {
    pub scope: EnvScope,
    pub key: TextInput,
    pub value: TextInput,
    pub field: EnvField,
    /// Key being edited (a rename removes it).
    pub original: Option<String>,
    pub error: Option<String>,
}

impl EnvForm {
    pub fn new(scope: EnvScope) -> Self {
        Self {
            scope,
            key: TextInput::default(),
            value: TextInput::default(),
            field: EnvField::Key,
            original: None,
            error: None,
        }
    }

    pub fn edit(scope: EnvScope, key: &str, value: &str) -> Self {
        Self {
            scope,
            key: TextInput::new(key),
            value: TextInput::new(value),
            field: EnvField::Value,
            original: Some(key.to_string()),
            error: None,
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextInput {
        match self.field {
            EnvField::Key => &mut self.key,
            EnvField::Value => &mut self.value,
        }
    }
}

// --- exe browser ------------------------------------------------------------

#[derive(Clone)]
pub enum BrowsePurpose {
    /// Change the exe of a registered app.
    AppExe(String),
    /// Pick the exe for the Add app form underneath.
    AddApp,
}

pub struct Browser {
    pub purpose: BrowsePurpose,
    pub cwd: PathBuf,
    /// Never navigate above this (wineprefix root); `None` = whole host FS.
    pub floor: Option<PathBuf>,
    pub entries: Vec<BrowseEntry>,
    pub has_parent: bool,
    pub sel: usize,
    pub error: Option<String>,
}

pub enum BrowseItem<'a> {
    Parent,
    Entry(&'a BrowseEntry),
}

impl Browser {
    pub fn open(purpose: BrowsePurpose, start: PathBuf, floor: Option<PathBuf>) -> Result<Self> {
        let mut b = Self {
            purpose,
            cwd: start,
            floor,
            entries: Vec::new(),
            has_parent: false,
            sel: 0,
            error: None,
        };
        b.reload()?;
        b.sel = b.first_entry();
        Ok(b)
    }

    pub fn reload(&mut self) -> Result<()> {
        self.has_parent = browse::can_go_up(&self.cwd, self.floor.as_deref());
        self.entries = browse::list_exe_browser(&self.cwd)?;
        self.sel = self.sel.min(self.len().saturating_sub(1));
        Ok(())
    }

    /// Cursor start: the first real entry rather than `../`.
    fn first_entry(&self) -> usize {
        usize::from(self.has_parent && !self.entries.is_empty())
    }

    pub fn len(&self) -> usize {
        self.entries.len() + usize::from(self.has_parent)
    }

    pub fn item(&self, idx: usize) -> Option<BrowseItem<'_>> {
        if self.has_parent {
            if idx == 0 {
                return Some(BrowseItem::Parent);
            }
            return self.entries.get(idx - 1).map(BrowseItem::Entry);
        }
        self.entries.get(idx).map(BrowseItem::Entry)
    }

    pub fn cd(&mut self, dir: PathBuf) -> Result<()> {
        let prev = std::mem::replace(&mut self.cwd, dir);
        self.sel = 0;
        if let Err(e) = self.reload() {
            // Unreadable dir (permissions): stay where we were.
            self.cwd = prev;
            self.reload()?;
            self.error = Some(e.to_string());
        } else {
            self.sel = self.first_entry();
            self.error = None;
        }
        Ok(())
    }

    pub fn up(&mut self) -> Result<()> {
        if !self.has_parent {
            return Ok(());
        }
        let Some(parent) = self.cwd.parent().map(Path::to_path_buf) else {
            return Ok(());
        };
        let from = self.cwd.clone();
        self.cd(parent)?;
        // Land on the directory we just left.
        if let Some(i) = self.entries.iter().position(|e| e.path == from) {
            self.sel = i + usize::from(self.has_parent);
        }
        Ok(())
    }
}

// --- command palette --------------------------------------------------------

pub struct Palette {
    pub input: TextInput,
    pub sel: usize,
}

pub struct Command {
    pub label: String,
    pub hint: String,
    pub action: Action,
}

impl Command {
    pub fn new(label: impl Into<String>, hint: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            action,
        }
    }
}
