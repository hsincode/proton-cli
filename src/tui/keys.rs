//! Key handling. The top overlay gets keys first; otherwise global keys
//! (tabs, palette, help, quit) and then the active tab.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::browse;
use crate::error::Result;

use super::app::{Action, App, Field, LibFocus, Setting, Tab, ToastKind};
use super::overlay::{
    locale_options, proton_options, AddField, AddForm, AppEnv, BrowseItem, BrowsePurpose, Browser,
    Command, Confirm, ConfirmAction, EnvField, EnvForm, EnvScope, InputDialog, InputTarget,
    Overlay, Palette, PickTarget, Picker, GAMESCOPE_OPTIONS, GRAPHICS_OPTIONS,
};
use super::text::{fuzzy_best, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

fn ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Movement keys usable while text is being typed (no bare j/k/g/G).
fn nav_typing(key: &KeyEvent) -> Option<Nav> {
    match key.code {
        KeyCode::Up => Some(Nav::Up),
        KeyCode::Down => Some(Nav::Down),
        KeyCode::PageUp => Some(Nav::PageUp),
        KeyCode::PageDown => Some(Nav::PageDown),
        KeyCode::Char('p' | 'k') if ctrl(key) => Some(Nav::Up),
        KeyCode::Char('n' | 'j') if ctrl(key) => Some(Nav::Down),
        _ => None,
    }
}

/// Movement keys in lists (vim keys included).
fn nav_list(key: &KeyEvent) -> Option<Nav> {
    if let Some(n) = nav_typing(key) {
        return Some(n);
    }
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Char('k') => Some(Nav::Up),
        KeyCode::Char('j') => Some(Nav::Down),
        KeyCode::Char('g') | KeyCode::Home => Some(Nav::Top),
        KeyCode::Char('G') | KeyCode::End => Some(Nav::Bottom),
        _ => None,
    }
}

fn apply_nav(sel: &mut usize, len: usize, nav: Nav) {
    if len == 0 {
        *sel = 0;
        return;
    }
    *sel = match nav {
        Nav::Up => (*sel + len - 1) % len,
        Nav::Down => (*sel + 1) % len,
        Nav::Top => 0,
        Nav::Bottom => len - 1,
        Nav::PageUp => sel.saturating_sub(10),
        Nav::PageDown => (*sel + 10).min(len - 1),
    };
}

fn is_left(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Left | KeyCode::Char('h')) && !ctrl(key)
}

fn is_right(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Right | KeyCode::Char('l')) && !ctrl(key)
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if ctrl(&key) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return Ok(());
    }
    if !app.overlays.is_empty() {
        return handle_overlay(app, key);
    }
    if app.tab == Tab::Library && app.lib.filtering {
        return handle_filter(app, key);
    }

    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(());
        }
        KeyCode::Char('?') => {
            app.overlays.push(Overlay::Help);
            return Ok(());
        }
        KeyCode::Char(':') => {
            open_palette(app);
            return Ok(());
        }
        KeyCode::Tab => {
            app.tab = app.tab.step(1);
            return Ok(());
        }
        KeyCode::BackTab => {
            app.tab = app.tab.step(-1);
            return Ok(());
        }
        KeyCode::Char(c @ '1'..='5') => {
            app.tab = Tab::ALL[(c as u8 - b'1') as usize];
            return Ok(());
        }
        _ => {}
    }

    match app.tab {
        Tab::Library => match app.lib.focus {
            LibFocus::List => library_list_key(app, key),
            LibFocus::Detail => library_detail_key(app, key),
        },
        Tab::Proton => proton_key(app, key),
        Tab::Prefixes => prefixes_key(app, key),
        Tab::Settings => settings_key(app, key),
        Tab::Doctor => doctor_key(app, key),
    }
}

// --- palette / actions --------------------------------------------------------

pub fn open_palette(app: &mut App) {
    app.overlays.push(Overlay::Palette(Palette {
        input: TextInput::default(),
        sel: 0,
    }));
}

/// Every palette entry, in default order.
pub fn commands(app: &App) -> Vec<Command> {
    let mut out = Vec::new();
    for id in app.visible_apps() {
        if let Some(e) = app.db.apps.get(&id) {
            out.push(Command::new(
                format!("Launch {}", e.name),
                id.clone(),
                Action::Launch(id),
            ));
        }
    }
    let has_app = app.selected_app_id().is_some();
    out.push(Command::new("Add app…", "Library", Action::AddApp));
    if has_app {
        out.push(Command::new(
            "Edit selected app",
            "Library",
            Action::EditApp,
        ));
        out.push(Command::new(
            "Remove selected app…",
            "Library",
            Action::RemoveApp,
        ));
    }
    out.extend([
        Command::new(
            "Install / update dwproton",
            "Proton",
            Action::InstallDwproton,
        ),
        Command::new(
            "Use latest dwproton as default",
            "Proton",
            Action::UseLatestDwproton,
        ),
        Command::new(
            "Toggle prefer managed / steam",
            "Proton",
            Action::TogglePrefer,
        ),
        Command::new("New prefix…", "Prefixes", Action::NewPrefix),
        Command::new("Change default locale…", "Settings", Action::GlobalLocale),
        Command::new(
            "Toggle graphics dxvk / wined3d",
            "Settings",
            Action::ToggleGraphics,
        ),
        Command::new(
            "Toggle Vulkan ICD sanitize",
            "Settings",
            Action::ToggleSanitize,
        ),
        Command::new("Cycle log level", "Settings", Action::CycleLogLevel),
        Command::new("Toggle gamescope", "Settings", Action::ToggleGamescope),
        Command::new(
            "Add environment variable…",
            "Settings",
            Action::AddGlobalEnv,
        ),
        Command::new(
            "Reset environment to defaults…",
            "Settings",
            Action::ResetGlobalEnv,
        ),
        Command::new("Run health check", "Doctor", Action::RunDoctor),
    ]);
    for tab in Tab::ALL {
        out.push(Command::new(
            format!("Go to {}", tab.title()),
            format!("{}", tab.index() + 1),
            Action::GoTab(tab),
        ));
    }
    out.push(Command::new("Keyboard shortcuts", "?", Action::Help));
    out.push(Command::new("Quit", "q", Action::Quit));
    out
}

/// Palette entries matching the query, best first.
pub fn filtered_commands(app: &App, query: &str) -> Vec<Command> {
    let mut scored: Vec<(usize, usize, Command)> = commands(app)
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| {
            fuzzy_best(query, &[c.label.as_str(), c.hint.as_str()]).map(|s| (s, i, c))
        })
        .collect();
    scored.sort_by_key(|(s, i, _)| (*s, *i));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

pub fn run_action(app: &mut App, action: Action) -> Result<()> {
    match action {
        Action::GoTab(t) => app.tab = t,
        Action::Launch(id) => {
            app.tab = Tab::Library;
            app.select_app(&id);
            app.launch(&id)?;
        }
        Action::AddApp => {
            app.tab = Tab::Library;
            app.overlays.push(Overlay::AddApp(AddForm::new()));
        }
        Action::EditApp => {
            app.tab = Tab::Library;
            if app.selected_app_id().is_some() {
                app.lib.focus = LibFocus::Detail;
            }
        }
        Action::RemoveApp => {
            app.tab = Tab::Library;
            confirm_remove_app(app);
        }
        Action::InstallDwproton => {
            app.tab = Tab::Proton;
            app.start_install();
        }
        Action::UseLatestDwproton => app.use_latest_dwproton()?,
        Action::TogglePrefer => app.toggle_prefer()?,
        Action::GlobalLocale => app.overlays.push(Overlay::Picker(Picker::global_locale(
            app.config.locale.as_deref(),
        ))),
        Action::ToggleGraphics => app.toggle_graphics()?,
        Action::ToggleSanitize => app.toggle_sanitize()?,
        Action::CycleLogLevel => app.cycle_log_level(1)?,
        Action::ToggleGamescope => app.toggle_gamescope()?,
        Action::AddGlobalEnv => {
            app.tab = Tab::Settings;
            app.overlays
                .push(Overlay::EnvEdit(EnvForm::new(EnvScope::Global)));
        }
        Action::ResetGlobalEnv => {
            app.tab = Tab::Settings;
            confirm_reset_env(app);
        }
        Action::NewPrefix => {
            app.tab = Tab::Prefixes;
            open_new_prefix(app);
        }
        Action::RunDoctor => {
            app.tab = Tab::Doctor;
            app.run_doctor()?;
        }
        Action::Help => app.overlays.push(Overlay::Help),
        Action::Quit => app.should_quit = true,
    }
    Ok(())
}

fn confirm_remove_app(app: &mut App) {
    let Some((id, e)) = app.selected_app() else {
        return;
    };
    let lines = vec![
        format!("{} ({id}) will be removed from the library.", e.name),
        format!("Its prefix `{}` and game files are kept.", e.prefix),
    ];
    app.overlays.push(Overlay::Confirm(Confirm {
        title: "Remove app?".into(),
        lines,
        action: ConfirmAction::RemoveApp(id),
    }));
}

fn confirm_reset_env(app: &mut App) {
    app.overlays.push(Overlay::Confirm(Confirm {
        title: "Reset environment?".into(),
        lines: vec![
            "All global variables are replaced with the built-ins:".into(),
            "DXVK_HUD=0, STEAMOS=1, UMU_USE_STEAM=1".into(),
        ],
        action: ConfirmAction::ResetGlobalEnv,
    }));
}

fn open_new_prefix(app: &mut App) {
    app.overlays.push(Overlay::Input(InputDialog {
        title: "New prefix".into(),
        label: "Id".into(),
        input: TextInput::new(app.next_prefix_name()),
        target: InputTarget::NewPrefix,
        error: None,
    }));
}

// --- library ------------------------------------------------------------------

fn handle_filter(app: &mut App, key: KeyEvent) -> Result<()> {
    let len = app.visible_apps().len();
    if let Some(nav) = nav_typing(&key) {
        apply_nav(&mut app.lib.sel, len, nav);
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            app.lib.filter.clear();
            app.lib.filtering = false;
            app.lib.sel = 0;
        }
        KeyCode::Enter => app.lib.filtering = false,
        _ => {
            let before = app.lib.filter.value().to_string();
            app.lib.filter.handle_key(key);
            if app.lib.filter.value() != before {
                app.lib.sel = 0;
            }
        }
    }
    Ok(())
}

fn library_list_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let len = app.visible_apps().len();
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut app.lib.sel, len, nav);
        return Ok(());
    }
    match key.code {
        KeyCode::Enter => {
            if let Some(id) = app.selected_app_id() {
                app.launch(&id)?;
            }
        }
        KeyCode::Char('e') | KeyCode::Right | KeyCode::Char('l')
            if app.selected_app_id().is_some() =>
        {
            app.lib.focus = LibFocus::Detail;
        }
        KeyCode::Char('a') => app.overlays.push(Overlay::AddApp(AddForm::new())),
        KeyCode::Char('d') | KeyCode::Delete => confirm_remove_app(app),
        KeyCode::Char('/') => app.lib.filtering = true,
        KeyCode::Esc if !app.lib.filter.is_empty() => {
            app.lib.filter.clear();
            app.lib.sel = 0;
        }
        KeyCode::Char('r') => {
            app.reload()?;
            app.notify(ToastKind::Info, "Reloaded");
        }
        _ => {}
    }
    Ok(())
}

fn library_detail_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(id) = app.selected_app_id() else {
        app.lib.focus = LibFocus::List;
        return Ok(());
    };
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut app.lib.field, Field::ALL.len(), nav);
        return Ok(());
    }
    let field = app.current_field();
    if field.cycles() && (is_left(&key) || is_right(&key)) {
        return app.cycle_field(if is_left(&key) { -1 } else { 1 });
    }
    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => app.lib.focus = LibFocus::List,
        KeyCode::Enter | KeyCode::Char(' ') => open_field_editor(app, &id, field),
        KeyCode::Char('a') => app.overlays.push(Overlay::AddApp(AddForm::new())),
        _ => {}
    }
    Ok(())
}

fn open_field_editor(app: &mut App, id: &str, field: Field) {
    let ov = match field {
        Field::Name => {
            let name = app
                .db
                .apps
                .get(id)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            Overlay::Input(InputDialog {
                title: "Rename".into(),
                label: "Name".into(),
                input: TextInput::new(name),
                target: InputTarget::AppName(id.to_string()),
                error: None,
            })
        }
        Field::Exe => Overlay::Picker(Picker::exe_source(app, id)),
        Field::Proton => Overlay::Picker(Picker::app_proton(app, id)),
        Field::Locale => Overlay::Picker(Picker::app_locale(app, id)),
        Field::Graphics => Overlay::Picker(Picker::app_graphics(app, id)),
        Field::Gamescope => Overlay::Picker(Picker::app_gamescope(app, id)),
        Field::Env => Overlay::AppEnv(AppEnv {
            id: id.to_string(),
            sel: 0,
        }),
    };
    app.overlays.push(ov);
}

// --- other tabs -----------------------------------------------------------------

fn proton_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut app.proton_sel, app.protons.len(), nav);
        return Ok(());
    }
    match key.code {
        KeyCode::Enter | KeyCode::Char(' ') => app.set_default_proton(app.proton_sel)?,
        KeyCode::Char('i') => app.start_install(),
        KeyCode::Char('a') => app.use_latest_dwproton()?,
        KeyCode::Char('p') => app.toggle_prefer()?,
        KeyCode::Char('r') => {
            app.reload()?;
            app.notify(ToastKind::Info, "Reloaded");
        }
        _ => {}
    }
    Ok(())
}

fn prefixes_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut app.prefix_sel, app.prefixes.len(), nav);
        return Ok(());
    }
    let selected = app.prefixes.get(app.prefix_sel).cloned();
    match key.code {
        KeyCode::Char('n') | KeyCode::Char('a') => open_new_prefix(app),
        KeyCode::Char('d') | KeyCode::Delete => {
            let Some(id) = selected else {
                return Ok(());
            };
            if id == crate::paths::DEFAULT_PREFIX_ID {
                app.notify(
                    ToastKind::Warn,
                    "The shared default prefix cannot be deleted",
                );
                return Ok(());
            }
            let users = app.prefix_users(&id);
            if !users.is_empty() {
                let names = users.join(", ");
                app.notify(ToastKind::Warn, format!("`{id}` is used by {names}"));
                return Ok(());
            }
            let path = app.paths.prefix_path(&id);
            app.overlays.push(Overlay::Confirm(Confirm {
                title: "Delete prefix?".into(),
                lines: vec![
                    format!("{} will be deleted from disk,", path.display()),
                    "including anything installed in it.".into(),
                ],
                action: ConfirmAction::DeletePrefix(id),
            }));
        }
        KeyCode::Char('s') => {
            if let Some(id) = selected {
                app.set_default_prefix(&id)?;
            }
        }
        KeyCode::Char('r') => {
            app.reload()?;
            app.notify(ToastKind::Info, "Reloaded");
        }
        _ => {}
    }
    Ok(())
}

fn settings_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(nav) = nav_list(&key) {
        let len = app.settings_len();
        apply_nav(&mut app.settings_sel, len, nav);
        return Ok(());
    }
    if key.code == KeyCode::Char('a') {
        app.overlays
            .push(Overlay::EnvEdit(EnvForm::new(EnvScope::Global)));
        return Ok(());
    }
    if key.code == KeyCode::Char('R') {
        confirm_reset_env(app);
        return Ok(());
    }

    let n_settings = Setting::ALL.len();
    let env_keys = app.env_keys();
    let sel = app.settings_sel;
    let (left, right) = (is_left(&key), is_right(&key));
    let activate = matches!(key.code, KeyCode::Enter | KeyCode::Char(' '));

    if sel < n_settings {
        let dir = if left { -1 } else { 1 };
        if !(left || right || activate) {
            return Ok(());
        }
        match Setting::ALL[sel] {
            Setting::Locale if activate => app.overlays.push(Overlay::Picker(
                Picker::global_locale(app.config.locale.as_deref()),
            )),
            Setting::Locale => app.cycle_global_locale(dir)?,
            Setting::Graphics => app.toggle_graphics()?,
            Setting::Sanitize => app.toggle_sanitize()?,
            Setting::LogLevel => app.cycle_log_level(dir)?,
            Setting::Gamescope => app.toggle_gamescope()?,
        }
    } else if let Some(k) = env_keys.get(sel - n_settings) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                let v = app.config.launch.env.get(k).cloned().unwrap_or_default();
                app.overlays
                    .push(Overlay::EnvEdit(EnvForm::edit(EnvScope::Global, k, &v)));
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let k = k.clone();
                app.delete_global_env(&k)?;
            }
            _ => {}
        }
    } else if activate {
        app.overlays
            .push(Overlay::EnvEdit(EnvForm::new(EnvScope::Global)));
    }
    Ok(())
}

fn doctor_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Rendered lines: up to two per check plus the paths section.
    let max = (app.checks.len() * 2 + 6) as u16;
    match nav_list(&key) {
        Some(Nav::Up) => app.doctor_scroll = app.doctor_scroll.saturating_sub(1),
        Some(Nav::Down) => app.doctor_scroll = (app.doctor_scroll + 1).min(max),
        Some(Nav::Top) => app.doctor_scroll = 0,
        Some(Nav::PageUp) => app.doctor_scroll = app.doctor_scroll.saturating_sub(10),
        Some(Nav::PageDown) => app.doctor_scroll = (app.doctor_scroll + 10).min(max),
        Some(Nav::Bottom) => app.doctor_scroll = max,
        None => {
            if key.code == KeyCode::Char('r') {
                app.run_doctor()?;
            }
        }
    }
    Ok(())
}

// --- overlays -------------------------------------------------------------------

enum Flow {
    Keep,
    Close,
}

fn handle_overlay(app: &mut App, key: KeyEvent) -> Result<()> {
    let depth = app.overlays.len();
    let Some(mut ov) = app.overlays.pop() else {
        return Ok(());
    };
    let flow = match &mut ov {
        Overlay::Picker(p) => picker_key(app, p, key),
        Overlay::Input(d) => input_key(app, d, key),
        Overlay::Confirm(c) => confirm_key(app, c, key),
        Overlay::AddApp(f) => add_key(app, f, key),
        Overlay::AppEnv(e) => app_env_key(app, e, key),
        Overlay::EnvEdit(f) => env_edit_key(app, f, key),
        Overlay::Browser(b) => browser_key(app, b, key),
        Overlay::Palette(p) => palette_key(app, p, key),
        Overlay::Help => Ok(Flow::Close),
    };
    match flow {
        // Anything the handler pushed stays above the kept overlay.
        Ok(Flow::Keep) => app.overlays.insert(depth - 1, ov),
        Ok(Flow::Close) => {}
        Err(e) => {
            app.overlays.insert(depth - 1, ov);
            return Err(e);
        }
    }
    Ok(())
}

fn picker_key(app: &mut App, p: &mut Picker, key: KeyEvent) -> Result<Flow> {
    let first_run = matches!(p.target, PickTarget::FirstRunLocale);
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut p.sel, p.items.len(), nav);
        return Ok(Flow::Keep);
    }
    match key.code {
        KeyCode::Char('q') if first_run => {
            app.should_quit = true;
            Ok(Flow::Keep)
        }
        KeyCode::Esc | KeyCode::Char('q') if !first_run => Ok(Flow::Close),
        KeyCode::Enter | KeyCode::Char(' ') => apply_pick(app, p),
        _ => Ok(Flow::Keep),
    }
}

fn apply_pick(app: &mut App, p: &Picker) -> Result<Flow> {
    let sel = p.sel;
    match &p.target {
        PickTarget::FirstRunLocale | PickTarget::GlobalLocale => app.set_global_locale(sel)?,
        PickTarget::AppProton(id) => {
            let v = proton_options(&app.protons).get(sel).cloned().flatten();
            app.set_app_proton(id, v)?;
        }
        PickTarget::AppLocale(id) => {
            let v = locale_options()
                .get(sel)
                .copied()
                .flatten()
                .map(str::to_string);
            app.set_app_locale(id, v)?;
        }
        PickTarget::AppGraphics(id) => {
            let v = GRAPHICS_OPTIONS
                .get(sel)
                .copied()
                .flatten()
                .map(str::to_string);
            app.set_app_graphics(id, v)?;
        }
        PickTarget::AppGamescope(id) => {
            let v = GAMESCOPE_OPTIONS.get(sel).copied().flatten();
            app.set_app_gamescope(id, v)?;
        }
        PickTarget::ExeSource(id) => {
            let Some(entry) = app.db.apps.get(id) else {
                return Ok(Flow::Close);
            };
            let purpose = BrowsePurpose::AppExe(id.clone());
            let browser = if sel == 0 {
                let prefix = app.paths.prefix_path(&entry.prefix);
                if !prefix.is_dir() {
                    app.notify(
                        ToastKind::Warn,
                        format!(
                            "Prefix not found: {} — launch once to create it",
                            prefix.display()
                        ),
                    );
                    return Ok(Flow::Keep);
                }
                Browser::open(purpose, browse::start_dir_for_prefix(&prefix), Some(prefix))?
            } else {
                Browser::open(purpose, browse::start_dir_for_filesystem(&entry.exe), None)?
            };
            app.overlays.push(Overlay::Browser(browser));
        }
    }
    Ok(Flow::Close)
}

fn input_key(app: &mut App, d: &mut InputDialog, key: KeyEvent) -> Result<Flow> {
    match key.code {
        KeyCode::Esc => Ok(Flow::Close),
        KeyCode::Enter => {
            let value = d.input.value().to_string();
            let res = match &d.target {
                InputTarget::AppName(id) => app.set_app_name(id, &value)?,
                InputTarget::NewPrefix => app.create_prefix(&value)?,
            };
            match res {
                Ok(()) => Ok(Flow::Close),
                Err(msg) => {
                    d.error = Some(msg);
                    Ok(Flow::Keep)
                }
            }
        }
        _ => {
            if d.input.handle_key(key) {
                d.error = None;
            }
            Ok(Flow::Keep)
        }
    }
}

fn confirm_key(app: &mut App, c: &mut Confirm, key: KeyEvent) -> Result<Flow> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            match &c.action {
                ConfirmAction::RemoveApp(id) => app.remove_app(id)?,
                ConfirmAction::DeletePrefix(id) => app.delete_prefix(id)?,
                ConfirmAction::ResetGlobalEnv => app.reset_global_env()?,
            }
            Ok(Flow::Close)
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
            Ok(Flow::Close)
        }
        _ => Ok(Flow::Keep),
    }
}

fn add_key(app: &mut App, f: &mut AddForm, key: KeyEvent) -> Result<Flow> {
    let save = (ctrl(&key) && key.code == KeyCode::Char('s'))
        || (f.field == AddField::Save && matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')));
    if save {
        return Ok(match app.register(f)? {
            Ok(_) => {
                app.lib.focus = LibFocus::List;
                Flow::Close
            }
            Err(msg) => {
                f.error = Some(msg);
                Flow::Keep
            }
        });
    }

    match key.code {
        KeyCode::Esc => return Ok(Flow::Close),
        KeyCode::Tab | KeyCode::Down => {
            f.field = f.field.step(1);
            return Ok(Flow::Keep);
        }
        KeyCode::BackTab | KeyCode::Up => {
            f.field = f.field.step(-1);
            return Ok(Flow::Keep);
        }
        KeyCode::Char('n' | 'j') if ctrl(&key) => {
            f.field = f.field.step(1);
            return Ok(Flow::Keep);
        }
        KeyCode::Char('p' | 'k') if ctrl(&key) => {
            f.field = f.field.step(-1);
            return Ok(Flow::Keep);
        }
        _ => {}
    }

    match f.field {
        AddField::Exe
            if key.code == KeyCode::Enter || (ctrl(&key) && key.code == KeyCode::Char('o')) =>
        {
            let start = browse::start_dir_for_filesystem(std::path::Path::new(f.exe.trimmed()));
            let browser = Browser::open(BrowsePurpose::AddApp, start, None)?;
            app.overlays.push(Overlay::Browser(browser));
        }
        AddField::Prefix
            if matches!(key.code, KeyCode::Left | KeyCode::Right)
                || (key.code == KeyCode::Char(' ')
                    && f.prefix_mode != super::overlay::PrefixMode::Custom) =>
        {
            f.prefix_mode = if key.code == KeyCode::Left {
                f.prefix_mode.prev()
            } else {
                f.prefix_mode.next()
            };
        }
        AddField::Proton
            if matches!(
                key.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l' | ' ')
            ) =>
        {
            let n = proton_options(&app.protons).len();
            let dir = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                -1
            } else {
                1
            };
            f.proton_sel = (f.proton_sel as i32 + dir).rem_euclid(n as i32) as usize;
        }
        _ if key.code == KeyCode::Enter => f.field = f.field.step(1),
        _ => {
            if let Some(t) = f.text_mut() {
                if t.handle_key(key) {
                    f.error = None;
                }
            }
        }
    }
    Ok(Flow::Keep)
}

fn app_env_key(app: &mut App, e: &mut AppEnv, key: KeyEvent) -> Result<Flow> {
    let Some(entry) = app.db.apps.get(&e.id) else {
        return Ok(Flow::Close);
    };
    let keys: Vec<String> = entry.env.keys().cloned().collect();
    if let Some(nav) = nav_list(&key) {
        apply_nav(&mut e.sel, keys.len() + 1, nav);
        return Ok(Flow::Keep);
    }
    let scope = EnvScope::App(e.id.clone());
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Ok(Flow::Close),
        KeyCode::Char('a') => app.overlays.push(Overlay::EnvEdit(EnvForm::new(scope))),
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') if e.sel == 0 => {
            app.toggle_app_custom_env(&e.id)?;
        }
        KeyCode::Enter | KeyCode::Char('e') => {
            if let Some(k) = keys.get(e.sel - 1) {
                let v = entry.env.get(k).cloned().unwrap_or_default();
                app.overlays
                    .push(Overlay::EnvEdit(EnvForm::edit(scope, k, &v)));
            }
        }
        KeyCode::Char('d') | KeyCode::Delete if e.sel > 0 => {
            if let Some(k) = keys.get(e.sel - 1).cloned() {
                app.delete_app_env(&e.id, &k)?;
                e.sel = e.sel.min(keys.len() - 1);
            }
        }
        _ => {}
    }
    Ok(Flow::Keep)
}

fn env_edit_key(app: &mut App, f: &mut EnvForm, key: KeyEvent) -> Result<Flow> {
    let switch = matches!(
        key.code,
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down
    ) || (ctrl(&key) && matches!(key.code, KeyCode::Char('n' | 'p' | 'j' | 'k')));
    if switch {
        f.field = match f.field {
            EnvField::Key => EnvField::Value,
            EnvField::Value => EnvField::Key,
        };
        return Ok(Flow::Keep);
    }
    let save = (ctrl(&key) && key.code == KeyCode::Char('s'))
        || (key.code == KeyCode::Enter && f.field == EnvField::Value);
    if save {
        let saved = match app.save_env(f)? {
            Ok(k) => k,
            Err(msg) => {
                f.error = Some(msg);
                return Ok(Flow::Keep);
            }
        };
        // Keep the cursor of the list underneath on the saved key.
        match &f.scope {
            EnvScope::Global => {
                if let Some(i) = app.env_keys().iter().position(|k| *k == saved) {
                    app.settings_sel = Setting::ALL.len() + i;
                }
            }
            EnvScope::App(id) => {
                let pos = app
                    .db
                    .apps
                    .get(id)
                    .and_then(|e| e.env.keys().position(|k| *k == saved));
                if let (Some(Overlay::AppEnv(list)), Some(i)) = (app.overlays.last_mut(), pos) {
                    list.sel = i + 1;
                }
            }
        }
        return Ok(Flow::Close);
    }
    match key.code {
        KeyCode::Esc => Ok(Flow::Close),
        KeyCode::Enter => {
            f.field = EnvField::Value;
            Ok(Flow::Keep)
        }
        _ => {
            if f.focused_mut().handle_key(key) {
                f.error = None;
            }
            Ok(Flow::Keep)
        }
    }
}

fn browser_key(app: &mut App, b: &mut Browser, key: KeyEvent) -> Result<Flow> {
    if let Some(nav) = nav_list(&key) {
        let n = b.len();
        apply_nav(&mut b.sel, n, nav);
        return Ok(Flow::Keep);
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => return Ok(Flow::Close),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => b.up()?,
        KeyCode::Char('~') if b.floor.is_none() => {
            if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
                b.cd(home)?;
            }
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            let target = match b.item(b.sel) {
                None => return Ok(Flow::Keep),
                Some(BrowseItem::Parent) => None,
                Some(BrowseItem::Entry(e)) => Some((e.path.clone(), e.is_dir)),
            };
            match target {
                None => b.up()?,
                Some((path, true)) => b.cd(path)?,
                Some((path, false)) => return pick_exe(app, b, path),
            }
        }
        _ => {}
    }
    Ok(Flow::Keep)
}

fn pick_exe(app: &mut App, b: &mut Browser, path: std::path::PathBuf) -> Result<Flow> {
    match b.purpose.clone() {
        BrowsePurpose::AppExe(id) => match app.set_app_exe(&id, path)? {
            Ok(()) => Ok(Flow::Close),
            Err(msg) => {
                b.error = Some(msg);
                Ok(Flow::Keep)
            }
        },
        BrowsePurpose::AddApp => {
            if !browse::is_windows_exe(&path) {
                b.error = Some(format!("Not a .exe: {}", path.display()));
                return Ok(Flow::Keep);
            }
            if let Some(Overlay::AddApp(form)) = app.overlays.last_mut() {
                form.exe.set(path.display().to_string());
                form.field = AddField::Name;
                form.error = None;
            }
            Ok(Flow::Close)
        }
    }
}

fn palette_key(app: &mut App, p: &mut Palette, key: KeyEvent) -> Result<Flow> {
    let matches = filtered_commands(app, p.input.value());
    if let Some(nav) = nav_typing(&key) {
        apply_nav(&mut p.sel, matches.len(), nav);
        return Ok(Flow::Keep);
    }
    match key.code {
        KeyCode::Esc => Ok(Flow::Close),
        KeyCode::Enter => {
            if let Some(cmd) = matches.into_iter().nth(p.sel) {
                // Close first so the action can open its own overlay.
                run_action(app, cmd.action)?;
            }
            Ok(Flow::Close)
        }
        _ => {
            let before = p.input.value().to_string();
            p.input.handle_key(key);
            if p.input.value() != before {
                p.sel = 0;
            }
            Ok(Flow::Keep)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale;

    #[test]
    fn nav_wraps_and_clamps() {
        let mut sel = 0;
        apply_nav(&mut sel, 3, Nav::Up);
        assert_eq!(sel, 2);
        apply_nav(&mut sel, 3, Nav::Down);
        assert_eq!(sel, 0);
        apply_nav(&mut sel, 3, Nav::PageDown);
        assert_eq!(sel, 2);
        apply_nav(&mut sel, 0, Nav::Down);
        assert_eq!(sel, 0);
    }

    #[test]
    fn locale_count_matches_options() {
        assert_eq!(locale_options().len(), locale::selectable().len() + 1);
    }
}
