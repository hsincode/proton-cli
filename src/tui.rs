//! Minimal setup TUI (launched when proton-cli is run with no subcommand).
//!
//! Covers the settings a player typically needs before launching games:
//! app list / launch, register app, wine prefixes, Proton runtime, doctor status.

use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;

use crate::apps::{new_entry, AppsDb};
use crate::config::Config;
use crate::desktop;
use crate::doctor;
use crate::error::{Error, Result};
use crate::launch;
use crate::paths::{is_valid_app_id, slugify, Paths, DEFAULT_PREFIX_ID};
use crate::proton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
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
    doctor_lines: Vec<String>,
    should_quit: bool,
}

const HOME_ITEMS: &[&str] = &[
    "Apps — list / launch",
    "Register app",
    "Wine prefixes",
    "Proton settings",
    "Doctor (health check)",
    "Quit",
];

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
        let mut app = Self {
            paths,
            config,
            screen: Screen::Home,
            home_idx: 0,
            apps_state: ListState::default(),
            prefix_state: ListState::default(),
            status: "↑↓ move  Enter select  Esc back  q quit".into(),
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
            doctor_lines: Vec::new(),
            should_quit: false,
        };
        app.refresh_lists()?;
        Ok(app)
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

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app)).map_err(Error::Io)?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(200)).map_err(Error::Io)? {
            if let Event::Key(key) = event::read().map_err(Error::Io)? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(app, key.code)?;
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    match app.screen {
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
                }
                1 => {
                    app.add_name.clear();
                    app.add_exe.clear();
                    app.add_id.clear();
                    app.add_prefix = app.config.default_prefix.clone();
                    app.add_proton = app.config.proton.default.clone();
                    app.add_field = AddField::Name;
                    app.screen = Screen::AddApp;
                }
                2 => {
                    app.refresh_lists()?;
                    app.screen = Screen::Prefixes;
                }
                3 => {
                    app.refresh_lists()?;
                    app.screen = Screen::Proton;
                }
                4 => {
                    app.refresh_doctor();
                    app.screen = Screen::Doctor;
                }
                5 => app.should_quit = true,
                _ => {}
            },
            _ => {}
        },
        Screen::Apps => match code {
            KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Home,
            KeyCode::Up | KeyCode::Char('k') => select_prev(&mut app.apps_state, app.app_ids.len()),
            KeyCode::Down | KeyCode::Char('j') => {
                select_next(&mut app.apps_state, app.app_ids.len())
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(i) = app.apps_state.selected() {
                    if let Some(id) = app.app_ids.get(i).cloned() {
                        let paths = app.paths.clone();
                        let config = app.config.clone();
                        suspend_tui_for_launch(|| {
                            let db = AppsDb::load(&paths)?;
                            let entry = db.get(&id)?;
                            let spec = launch::LaunchSpec::from_app(&paths, &config, entry)?;
                            let _ = launch::run_umu(&config, &spec)?;
                            Ok(())
                        })?;
                        app.status = format!("launched `{id}` (returned)");
                    }
                }
            }
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
            KeyCode::Esc => app.screen = Screen::Home,
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
            KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Home,
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
            KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Home,
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
                suspend_tui_for_launch(|| {
                    println!("Installing dwproton…");
                    let path = proton::install_dwproton(&paths, None, false)?;
                    println!("Installed: {}", path.display());
                    Ok(())
                })?;
                app.refresh_lists()?;
                app.status = "dwproton install finished".into();
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
            KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Home,
            KeyCode::Char('r') => {
                app.refresh_doctor();
                app.status = "doctor refreshed".into();
            }
            _ => {}
        },
    }
    Ok(())
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
fn suspend_tui_for_launch(action: impl FnOnce() -> Result<()>) -> Result<()> {
    disable_raw_mode().ok();
    let mut out = stdout();
    execute!(out, LeaveAlternateScreen).ok();
    let res = action();
    enable_raw_mode().map_err(Error::Io)?;
    execute!(out, EnterAlternateScreen).map_err(Error::Io)?;
    res
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

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " proton-cli ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  root={}  ", app.paths.root.display())),
        Span::raw(match app.screen {
            Screen::Home => "Home",
            Screen::Apps => "Apps",
            Screen::AddApp => "Register",
            Screen::Prefixes => "Prefixes",
            Screen::Proton => "Proton",
            Screen::Doctor => "Doctor",
        }),
    ]))
    .block(Block::default().borders(Borders::ALL).title("setup TUI"));
    f.render_widget(title, chunks[0]);

    match app.screen {
        Screen::Home => {
            let items: Vec<ListItem> = HOME_ITEMS
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let prefix = if i == app.home_idx { "> " } else { "  " };
                    ListItem::new(format!("{prefix}{label}"))
                })
                .collect();
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Main menu — gaming setup"),
            );
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
                            format!(
                                "{id}  —  {}  pfx={}  {}",
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
                .block(Block::default().borders(Borders::ALL).title(
                    "Apps  [Enter/l launch]  [d remove]  [r refresh]  [Esc back]",
                ))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.apps_state);
        }
        Screen::AddApp => {
            let mark = |fld: AddField| if app.add_field == fld { ">" } else { " " };
            let id_hint = if app.add_id.is_empty() {
                format!("(auto: {})", slugify(&app.add_name))
            } else {
                String::new()
            };
            let text = format!(
                "{m0} Name   : {name}\n\
                 {m1} Exe    : {exe}\n\
                 {m2} Id     : {id} {id_hint}\n\
                 {m3} Prefix : {prefix}\n\
                 {m4} Proton : {proton}\n\n\
                 Prefix: `default` → ~/.protoncli/default\n\
                 other id → ~/.protoncli/prefixes/<id>\n\
                 Tab field · type · Enter save · Esc cancel",
                m0 = mark(AddField::Name),
                m1 = mark(AddField::Exe),
                m2 = mark(AddField::Id),
                m3 = mark(AddField::Prefix),
                m4 = mark(AddField::Proton),
                name = app.add_name,
                exe = app.add_exe,
                id = app.add_id,
                prefix = app.add_prefix,
                proton = app.add_proton,
            );
            let p = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Register app"))
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
        }
        Screen::Prefixes => {
            let items: Vec<ListItem> = app
                .prefix_ids
                .iter()
                .map(|id| {
                    let path = app.paths.prefix_path(id);
                    let marker = if id == &app.config.default_prefix {
                        " [config default]"
                    } else {
                        ""
                    };
                    ListItem::new(format!("{id}  →  {}{marker}", path.display()))
                })
                .collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(
                    "Prefixes  [c create pfx-N]  [s set default]  [Enter path]  [Esc]",
                ))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            f.render_stateful_widget(list, chunks[1], &mut app.prefix_state);
        }
        Screen::Proton => {
            let mut lines = vec![
                format!(
                    "config: default={}  prefer={}",
                    app.config.proton.default, app.config.proton.prefer
                ),
                "keys: [i] install dwproton  [m] managed  [t] steam  [d] default=dwproton  [r] refresh"
                    .into(),
                String::new(),
                "Installed:".into(),
            ];
            if app.proton_names.is_empty() {
                lines.push("  (none — press i to install)".into());
            } else {
                for (i, name) in app.proton_names.iter().enumerate() {
                    let sel = app.proton_sel == i;
                    lines.push(format!("{} {name}", if sel { ">" } else { " " }));
                }
            }
            let p = Paragraph::new(lines.join("\n"))
                .block(Block::default().borders(Borders::ALL).title("Proton"))
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
        }
        Screen::Doctor => {
            let p = Paragraph::new(app.doctor_lines.join("\n"))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Doctor  [r refresh]  [Esc back]"),
                )
                .wrap(Wrap { trim: false });
            f.render_widget(p, chunks[1]);
        }
    }

    let status = Paragraph::new(app.status.as_str())
        .block(Block::default().borders(Borders::ALL).title("status"));
    f.render_widget(status, chunks[2]);
}
