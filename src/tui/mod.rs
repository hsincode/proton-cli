//! Interactive TUI (`protoncli` with no subcommand, or `protoncli tui`).
//!
//! Layout: a tab bar (Library / Proton / Prefixes / Settings / Doctor), the
//! active tab, a status line for toasts and context help, and a key bar.
//! Library is the home tab: apps on the left, the selected app's effective
//! settings on the right (edited in place). `:` opens a command palette that
//! also launches apps; `?` lists keys.
//!
//! Modules:
//! - `app`      state and read-only queries
//! - `actions`  mutations (save to disk, then reload)
//! - `keys`     key handling
//! - `view` / `view_overlay`  rendering
//! - `overlay`  modal layer types (pickers, forms, browser, palette)
//! - `checks`   health checks for the Doctor tab
//! - `text`     text input + fuzzy matching

mod actions;
mod app;
mod checks;
mod keys;
mod overlay;
mod text;
mod theme;
mod view;
mod view_overlay;

use std::io::{self, stdout};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::paths::Paths;

use app::{App, ToastKind};

pub fn run(paths: Paths, config: Config) -> Result<()> {
    // Release builds use panic = "abort", so no unwinding cleanup runs.
    // Restore the terminal from the panic hook instead.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_raw_mode().ok();
        execute!(stdout(), LeaveAlternateScreen, Show).ok();
        default_hook(info);
    }));

    enable_raw_mode().map_err(Error::Io)?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).map_err(Error::Io)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out)).map_err(Error::Io)?;

    let result = App::new(paths, config).and_then(|mut app| event_loop(&mut terminal, &mut app));

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        if app.needs_full_redraw {
            // After a game or installer owned the tty, ratatui's buffer no
            // longer matches the screen: clear and drop queued keystrokes.
            terminal.clear().map_err(Error::Io)?;
            while event::poll(Duration::ZERO).unwrap_or(false) {
                let _ = event::read();
            }
            app.needs_full_redraw = false;
        }

        if let Err(e) = app.poll_task() {
            app.notify(ToastKind::Err, e.to_string());
        }
        app.expire_toast();
        terminal.draw(|f| view::draw(f, app)).map_err(Error::Io)?;
        if app.should_quit {
            return Ok(());
        }

        // Tick faster while a spinner / progress bar is visible.
        let timeout = if app.task.is_some() {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        };
        app.tick = app.tick.wrapping_add(1);
        if !event::poll(timeout).map_err(Error::Io)? {
            continue;
        }
        match event::read().map_err(Error::Io)? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // A failed action is reported, not fatal.
                if let Err(e) = keys::handle_key(app, key) {
                    app.notify(ToastKind::Err, e.to_string());
                }
            }
            Event::Resize(..) => app.needs_full_redraw = true,
            _ => {}
        }
    }
}

/// Leave the TUI while `action` owns the terminal (game launch), then return.
///
/// The caller must set `needs_full_redraw` afterwards.
fn suspend(action: impl FnOnce() -> Result<()>) -> Result<()> {
    disable_raw_mode().ok();
    let mut out = stdout();
    execute!(out, LeaveAlternateScreen, Show).ok();
    let res = action();
    execute!(out, EnterAlternateScreen, Hide).map_err(Error::Io)?;
    enable_raw_mode().map_err(Error::Io)?;
    res
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use tempfile::{tempdir, TempDir};

    use super::app::{App, LibFocus, Tab};
    use super::overlay::Overlay;
    use super::{keys, view};
    use crate::apps::{new_entry, AppsDb};
    use crate::config::Config;
    use crate::paths::Paths;

    fn test_paths(root: &Path) -> Paths {
        Paths {
            root: root.to_path_buf(),
            config_dir: root.to_path_buf(),
            data_dir: root.to_path_buf(),
            config_file: root.join("config.toml"),
            apps_file: root.join("apps.toml"),
            prefixes_dir: root.join("prefixes"),
            default_prefix: root.join("default"),
            runtimes_dir: root.join("runtimes"),
            icons_dir: root.join("icons"),
            applications_dir: root.join("applications"),
        }
    }

    /// Two apps (one with a missing prefix) and a managed dwproton build.
    fn fixture() -> (TempDir, App) {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        let paths = test_paths(&root);
        paths.ensure_dirs().unwrap();
        let proton = paths.runtimes_dir.join("dwproton-11.0-15-x86_64");
        fs::create_dir_all(&proton).unwrap();
        fs::write(proton.join("proton"), "").unwrap();

        let games = dir.path().join("games");
        fs::create_dir_all(&games).unwrap();
        let mut db = AppsDb::default();
        for (id, name, prefix) in [
            ("elden-ring", "Elden Ring", "default"),
            ("old-game", "Old Game", "old-game"),
        ] {
            let exe = games.join(format!("{id}.exe"));
            fs::write(&exe, "").unwrap();
            let e = new_entry(
                name.into(),
                exe,
                vec![],
                None,
                prefix.into(),
                None,
                None,
                None,
                false,
            )
            .unwrap();
            db.insert(id.into(), e).unwrap();
        }
        db.save(&paths).unwrap();

        let mut config = Config::default();
        config.locale = Some("ja".into());
        config.save(&paths).unwrap();
        let app = App::new(paths, config).unwrap();
        (dir, app)
    }

    fn press(app: &mut App, code: KeyCode) {
        let key = KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        keys::handle_key(app, key).unwrap();
    }

    fn type_text(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| view::draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..h {
            let mut skip = 0;
            for x in 0..w {
                // Wide glyphs occupy two cells; the second is a blank placeholder.
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                skip = ratatui::text::Span::raw(sym).width().saturating_sub(1);
                out.push_str(sym);
            }
            out.push('\n');
        }
        if std::env::var_os("DUMP_TUI").is_some() {
            println!("{out}");
        }
        out
    }

    #[test]
    fn every_tab_renders_at_common_sizes() {
        let (_dir, mut app) = fixture();
        for (w, h) in [(120, 32), (80, 24), (60, 18)] {
            for tab in Tab::ALL {
                app.tab = tab;
                let screen = render(&app, w, h);
                assert!(screen.contains(tab.title()), "{tab:?} at {w}x{h}");
            }
        }
    }

    #[test]
    fn library_shows_effective_settings_and_problems() {
        let (_dir, app) = fixture();
        let screen = render(&app, 120, 32);
        assert!(screen.contains("Elden Ring"));
        assert!(screen.contains("no prefix"), "missing prefix is flagged");
        assert!(screen.contains("dwproton → dwproton-11.0-15-x86_64"));
        assert!(screen.contains("global"));
    }

    #[test]
    fn filter_narrows_the_list() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "old");
        assert_eq!(app.visible_apps(), vec!["old-game".to_string()]);
        press(&mut app, KeyCode::Enter);
        assert!(!app.lib.filtering);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible_apps().len(), 2);
    }

    #[test]
    fn detail_arrows_override_and_restore_settings() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.lib.focus, LibFocus::Detail);
        // Name, Exe, Proton, Locale, Graphics
        for _ in 0..4 {
            press(&mut app, KeyCode::Down);
        }
        press(&mut app, KeyCode::Right);
        let db = AppsDb::load(&app.paths).unwrap();
        assert_eq!(db.apps["elden-ring"].graphics.as_deref(), Some("dxvk"));
        assert!(render(&app, 120, 32).contains("override"));
        press(&mut app, KeyCode::Left);
        let db = AppsDb::load(&app.paths).unwrap();
        assert_eq!(db.apps["elden-ring"].graphics, None);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.lib.focus, LibFocus::List);
    }

    #[test]
    fn rename_through_dialog() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.overlays.last(), Some(Overlay::Input(_))));
        for _ in 0.."Elden Ring".len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        // Empty names are rejected inline.
        assert!(matches!(app.overlays.last(), Some(Overlay::Input(d)) if d.error.is_some()));
        type_text(&mut app, "ELDEN RING");
        press(&mut app, KeyCode::Enter);
        assert!(app.overlays.is_empty());
        assert_eq!(app.db.apps["elden-ring"].name, "ELDEN RING");
    }

    #[test]
    fn remove_asks_for_confirmation() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.overlays.last(), Some(Overlay::Confirm(_))));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.db.apps.len(), 2);
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.db.apps.len(), 1);
    }

    #[test]
    fn add_app_with_typed_exe_path() {
        let (dir, mut app) = fixture();
        let exe = dir.path().join("games/new-game.exe");
        fs::write(&exe, "").unwrap();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, &exe.display().to_string());
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "New Game");
        render(&app, 100, 30);
        let save = KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        keys::handle_key(&mut app, save).unwrap();
        assert!(app.overlays.is_empty());
        assert_eq!(app.db.apps["new-game"].name, "New Game");
        assert_eq!(app.selected_app_id().as_deref(), Some("new-game"));
    }

    #[test]
    fn palette_runs_commands() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, "go to doctor");
        render(&app, 100, 30);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tab, Tab::Doctor);
        assert!(app.overlays.is_empty());
    }

    #[test]
    fn settings_toggle_and_env_edit() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Down); // Graphics
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.config.launch.graphics, "wined3d");
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "FOO");
        press(&mut app, KeyCode::Enter); // key → value
        type_text(&mut app, "bar baz");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.config.launch.env.get("FOO").map(String::as_str),
            Some("bar baz")
        );
        let saved = Config::load(&app.paths).unwrap();
        assert_eq!(
            saved.launch.env.get("FOO").map(String::as_str),
            Some("bar baz")
        );
        assert!(render(&app, 100, 30).contains("FOO"));
    }

    #[test]
    fn first_run_requires_a_locale() {
        let (_dir, app) = fixture();
        let mut config = app.config.clone();
        config.locale = None;
        let mut app = App::new(app.paths.clone(), config).unwrap();
        assert!(matches!(app.overlays.last(), Some(Overlay::Picker(_))));
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            app.overlays.len(),
            1,
            "Esc cannot skip the first-run picker"
        );
        press(&mut app, KeyCode::Enter);
        assert!(app.overlays.is_empty());
        assert!(app.config.locale.is_some());
    }

    #[test]
    fn prefixes_create_and_refuse_used_delete() {
        let (_dir, mut app) = fixture();
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Enter);
        assert!(app.prefixes.contains(&"pfx-1".to_string()));
        // `default` cannot be deleted; nothing is asked.
        app.prefix_sel = 0;
        press(&mut app, KeyCode::Char('d'));
        assert!(app.overlays.is_empty());
    }
}
