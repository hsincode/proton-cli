//! Rendering of the tab bar, tabs, status line and key bar.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, LineGauge, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config;
use crate::launch;
use crate::locale;

use super::app::{App, Field, LibFocus, Setting, Tab, ToastKind};
use super::checks::Level;
use super::theme;
use super::view_overlay;

pub fn draw(f: &mut Frame, app: &App) {
    let [top, body, status, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_top_bar(f, app, top);
    match app.tab {
        Tab::Library => draw_library(f, app, body),
        Tab::Proton => draw_proton(f, app, body),
        Tab::Prefixes => draw_prefixes(f, app, body),
        Tab::Settings => draw_settings(f, app, body),
        Tab::Doctor => draw_doctor(f, app, body),
    }
    draw_status(f, app, status);
    draw_keybar(f, app, keys);

    let n = app.overlays.len();
    for (i, ov) in app.overlays.iter().enumerate() {
        view_overlay::draw(f, app, ov, i + 1 == n);
    }
}

// --- helpers ------------------------------------------------------------------

pub fn width(s: &str) -> usize {
    Span::raw(s).width()
}

/// Keep the end of `s` (paths: the file name matters most).
pub fn truncate_left(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 1; // room for the ellipsis
    for c in s.chars().rev() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
}

pub fn truncate_right(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 1;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// `left` then `right` pushed to the far edge of a `total`-wide row.
pub fn spread<'a>(mut left: Vec<Span<'a>>, right: Vec<Span<'a>>, total: usize) -> Line<'a> {
    let lw: usize = left.iter().map(Span::width).sum();
    let rw: usize = right.iter().map(Span::width).sum();
    if lw + rw < total {
        left.push(Span::raw(" ".repeat(total - lw - rw)));
        left.extend(right);
    }
    Line::from(left)
}

pub fn pane<'a>(title: impl Into<String>, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            theme::border_focus()
        } else {
            theme::border()
        })
        .title(Span::styled(
            format!(" {} ", title.into()),
            theme::pane_title(),
        ))
}

fn centered_message(f: &mut Frame, area: Rect, lines: Vec<Line>) {
    let h = lines.len() as u16;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect::new(area.x, y, area.width, h.min(area.height));
    f.render_widget(Paragraph::new(lines).centered(), rect);
}

fn level_icon(level: Level) -> Span<'static> {
    match level {
        Level::Ok => Span::styled("✓", theme::ok()),
        Level::Info => Span::styled("·", theme::dim()),
        Level::Warn => Span::styled("⚠", theme::warn()),
        Level::Err => Span::styled("✕", theme::err()),
    }
}

// --- chrome -------------------------------------------------------------------

fn draw_top_bar(f: &mut Frame, app: &App, area: Rect) {
    // Narrow terminals: inactive tabs collapse to their number.
    let compact = area.width < 96;
    let mut left = vec![Span::styled(" protoncli ", theme::brand()), Span::raw(" ")];
    for tab in Tab::ALL {
        let (label, style) = if tab == app.tab {
            (
                format!("{} {}", tab.index() + 1, tab.title()),
                theme::tab_active(),
            )
        } else if compact {
            (format!("{}", tab.index() + 1), theme::tab_idle())
        } else {
            (
                format!("{} {}", tab.index() + 1, tab.title()),
                theme::tab_idle(),
            )
        };
        left.push(Span::styled(label, style));
        left.push(Span::raw("  "));
    }

    let right = if let Some(task) = &app.task {
        let pct = task
            .ratio()
            .map(|r| format!(" {:>3.0}%", r * 100.0))
            .unwrap_or_default();
        vec![Span::styled(
            format!("{} Installing dwproton{pct} ", spinner(app.tick)),
            theme::accent(),
        )]
    } else {
        let issues = app.issue_count();
        if issues == 0 {
            vec![Span::styled("✓ ready ", theme::ok())]
        } else {
            let style = if app.has_errors() {
                theme::err()
            } else {
                theme::warn()
            };
            let noun = if issues == 1 { "issue" } else { "issues" };
            vec![Span::styled(format!("⚠ {issues} {noun} "), style)]
        }
    };
    f.render_widget(
        Paragraph::new(spread(left, right, area.width as usize)),
        area,
    );
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// `" Downloading x.tar.xz  120 / 410 MiB "`
fn gauge_label(task: &super::app::InstallTask) -> String {
    match task.total {
        Some(total) => format!(" {}  {} / {} ", task.stage, mib(task.done), mib(total)),
        None => format!(" {} ", task.stage),
    }
}

pub fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[(tick % FRAMES.len() as u64) as usize]
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(t) = &app.toast {
        let (icon, style) = match t.kind {
            ToastKind::Info => ("·", theme::text()),
            ToastKind::Ok => ("✓", theme::ok()),
            ToastKind::Warn => ("⚠", theme::warn()),
            ToastKind::Err => ("✕", theme::err()),
        };
        Line::from(vec![
            Span::raw(" "),
            Span::styled(icon, style),
            Span::raw(" "),
            Span::styled(t.text.clone(), style),
        ])
    } else if app.overlays.is_empty() {
        let help = match app.tab {
            Tab::Library if app.lib.focus == LibFocus::Detail => app.current_field().help(),
            Tab::Settings => Setting::ALL
                .get(app.settings_sel)
                .map(|s| s.help())
                .unwrap_or("Variables passed to every launch; apps can add their own"),
            _ => "",
        };
        Line::from(Span::styled(format!(" {help}"), theme::dim()))
    } else {
        Line::default()
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Key hints for the current context: (key, label).
pub fn key_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.overlays.last() {
        Some(ov) => view_overlay::key_hints(ov),
        None => tab_key_hints(app),
    }
}

/// Key hints of the active tab, ignoring overlays.
pub fn tab_key_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut hints: Vec<(&str, &str)> = match app.tab {
        Tab::Library if app.lib.filtering => {
            return vec![
                ("type", "filter"),
                ("↑↓", "move"),
                ("⏎", "done"),
                ("esc", "clear"),
            ]
        }
        Tab::Library if app.lib.focus == LibFocus::Detail => {
            vec![
                ("↑↓", "field"),
                ("←→", "change"),
                ("⏎", "edit"),
                ("esc", "back"),
            ]
        }
        Tab::Library => vec![
            ("⏎", "launch"),
            ("e", "edit"),
            ("a", "add"),
            ("d", "remove"),
            ("/", "filter"),
        ],
        Tab::Proton => vec![
            ("⏎", "set default"),
            ("i", "install/update dwproton"),
            ("a", "use latest"),
            ("p", "prefer"),
        ],
        Tab::Prefixes => vec![("n", "new"), ("d", "delete"), ("s", "set default")],
        Tab::Settings => vec![
            ("←→", "change"),
            ("a", "add var"),
            ("d", "delete var"),
            ("R", "reset env"),
        ],
        Tab::Doctor => vec![("r", "re-run"), ("↑↓", "scroll")],
    };
    hints.extend([
        ("tab", "next tab"),
        (":", "commands"),
        ("?", "help"),
        ("q", "quit"),
    ]);
    hints
}

fn draw_keybar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (k, label) in key_hints(app) {
        let w = width(k) + 1 + width(label) + 3;
        if used + w > area.width as usize {
            break;
        }
        used += w;
        spans.push(Span::styled(k, theme::key()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, theme::key_label()));
        spans.push(Span::raw("   "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// --- library ------------------------------------------------------------------

fn draw_library(f: &mut Frame, app: &App, area: Rect) {
    let (list_area, detail_area) = if area.width >= 90 {
        let w = (area.width * 38 / 100).max(32);
        let [a, b] = Layout::horizontal([Constraint::Length(w), Constraint::Min(30)]).areas(area);
        (a, b)
    } else {
        let [a, b] = Layout::vertical([Constraint::Percentage(45), Constraint::Min(8)]).areas(area);
        (a, b)
    };
    draw_app_list(f, app, list_area);
    draw_app_detail(f, app, detail_area);
}

fn draw_app_list(f: &mut Frame, app: &App, area: Rect) {
    let ids = app.visible_apps();
    let total = app.db.apps.len();
    let title = if app.lib.filter.is_empty() {
        format!("Apps {total}")
    } else {
        format!("Apps {}/{total}", ids.len())
    };
    let focused = app.lib.focus == LibFocus::List && app.overlays.is_empty();
    let block = pane(title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [filter_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    let filter_line = if app.lib.filtering || !app.lib.filter.is_empty() {
        Line::from(vec![
            Span::styled("  / ", theme::accent()),
            Span::raw(app.lib.filter.value().to_string()),
        ])
    } else {
        Line::from(Span::styled("  / filter", theme::dim()))
    };
    f.render_widget(Paragraph::new(filter_line), filter_area);
    if app.lib.filtering && app.overlays.is_empty() {
        f.set_cursor_position((
            filter_area.x + 4 + app.lib.filter.cursor_width(),
            filter_area.y,
        ));
    }

    if total == 0 {
        centered_message(
            f,
            list_area,
            vec![
                Line::from(Span::styled("No apps yet", theme::bold())),
                Line::from(vec![
                    Span::styled("a", theme::key()),
                    Span::styled(" to add a Windows game", theme::dim()),
                ]),
            ],
        );
        return;
    }
    if ids.is_empty() {
        centered_message(
            f,
            list_area,
            vec![Line::from(Span::styled("No matches", theme::dim()))],
        );
        return;
    }

    let w = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = ids
        .iter()
        .filter_map(|id| app.db.apps.get(id).map(|e| (id, e)))
        .map(|(_id, e)| {
            let problem = if !e.exe.is_file() {
                Some(Span::styled("no exe ", theme::err()))
            } else if e
                .proton
                .as_deref()
                .is_some_and(|p| !p.is_empty() && app.resolved(p).is_none())
            {
                Some(Span::styled("no proton ", theme::err()))
            } else if !app.prefix_exists(e) {
                Some(Span::styled("no prefix ", theme::warn()))
            } else {
                None
            };
            let rw = problem.as_ref().map(Span::width).unwrap_or(0);
            let name = truncate_right(&e.name, w.saturating_sub(rw + 1));
            ListItem::new(spread(
                vec![Span::raw(name)],
                problem.into_iter().collect(),
                w,
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.lib.sel));
    let list = List::new(items)
        .highlight_symbol("▌ ")
        .highlight_style(if focused {
            theme::selected()
        } else {
            theme::selected_idle()
        });
    f.render_stateful_widget(list, list_area, &mut state);
}

/// (value, source tag, value style) for a detail row.
fn field_value(
    app: &App,
    e: &crate::apps::AppEntry,
    field: Field,
) -> (String, Span<'static>, Style) {
    let global = || Span::styled("global", theme::dim());
    let over = || Span::styled("override", theme::overridden());
    match field {
        Field::Name => (e.name.clone(), Span::raw(""), theme::bold()),
        Field::Exe => {
            let ok = e.exe.is_file();
            (
                e.exe.display().to_string(),
                if ok {
                    Span::styled("✓", theme::ok())
                } else {
                    Span::styled("missing", theme::err())
                },
                if ok { theme::text() } else { theme::err() },
            )
        }
        Field::Proton => {
            let (req, tag) = match e.proton.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => (p, over()),
                None => (app.config.proton.default.as_str(), global()),
            };
            let style = if app.resolved(req).is_some() {
                theme::text()
            } else {
                theme::err()
            };
            (app.describe_proton(req), tag, style)
        }
        Field::Locale => {
            let (id, tag) = match e.locale.as_deref() {
                Some(l) => (Some(l), over()),
                None => (app.config.locale.as_deref(), global()),
            };
            let value = match id.and_then(locale::find) {
                Some(l) => format!("{} · {}", l.label, l.posix),
                None => "not set".into(),
            };
            (value, tag, theme::text())
        }
        Field::Graphics => {
            let g = launch::effective_graphics(e.graphics.as_deref(), &app.config.launch.graphics);
            let tag = if e.graphics.is_some() {
                over()
            } else {
                global()
            };
            (g.to_string(), tag, theme::text())
        }
        Field::Gamescope => {
            let g = launch::effective_gamescope(e.gamescope, &app.config.launch.gamescope);
            let tag = if e.gamescope.is_some() {
                over()
            } else {
                global()
            };
            (g.status_label(), tag, theme::text())
        }
        Field::Env => {
            let n_global = app.config.launch.env.len();
            if e.use_custom_env {
                (
                    format!("{n_global} global + {} custom", e.env.len()),
                    over(),
                    theme::text(),
                )
            } else {
                (format!("{n_global} global"), global(), theme::text())
            }
        }
    }
}

fn draw_app_detail(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.lib.focus == LibFocus::Detail && app.overlays.is_empty();
    let Some((id, e)) = app.selected_app() else {
        f.render_widget(pane("Details", false), area);
        return;
    };
    let block = pane(e.name.clone(), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(if focused {
        Line::from(Span::styled(
            " Editing — changes save immediately",
            theme::dim(),
        ))
    } else {
        Line::from(vec![
            Span::styled(" ▶ ", theme::ok()),
            Span::styled("⏎", theme::key()),
            Span::styled(" launch   ", theme::dim()),
            Span::styled("e", theme::key()),
            Span::styled(" edit", theme::dim()),
        ])
    });
    lines.push(Line::default());

    const LABEL_W: usize = 11;
    for (i, field) in Field::ALL.iter().enumerate() {
        let (value, tag, style) = field_value(app, e, *field);
        let selected = focused && i == app.lib.field;
        let value_w = w.saturating_sub(LABEL_W + tag.width() + 6);
        let value = if *field == Field::Exe {
            truncate_left(&value, value_w)
        } else {
            truncate_right(&value, value_w)
        };
        let (open, close) = if selected && field.cycles() {
            ("‹ ", " ›")
        } else {
            ("  ", "  ")
        };
        let mut line = spread(
            vec![
                Span::styled(format!(" {:<LABEL_W$}", field.label()), theme::dim()),
                Span::styled(open, theme::accent()),
                Span::styled(value, style),
                Span::styled(close, theme::accent()),
            ],
            vec![tag, Span::raw(" ")],
            w + 2,
        );
        if selected {
            line = line.style(theme::selected());
        }
        lines.push(line);
    }

    lines.push(Line::default());
    let prefix_path = app.paths.prefix_path(&e.prefix);
    let prefix_state = if prefix_path.is_dir() {
        Span::styled("✓", theme::ok())
    } else {
        Span::styled("missing — created on launch", theme::warn())
    };
    let info = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!(" {label:<LABEL_W$}  "), theme::dim()),
            Span::styled(
                truncate_left(&value, w.saturating_sub(LABEL_W + 3)),
                theme::dim(),
            ),
        ])
    };
    lines.push(info("Id", id.clone()));
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<LABEL_W$}  ", "Prefix"), theme::dim()),
        Span::styled(format!("{} ", e.prefix), theme::dim()),
        prefix_state,
    ]));
    lines.push(info("", prefix_path.display().to_string()));
    lines.push(info(
        "Shortcut",
        if e.desktop {
            "application menu".into()
        } else {
            "none".into()
        },
    ));
    lines.push(info("Added", e.created_at.format("%Y-%m-%d").to_string()));

    f.render_widget(Paragraph::new(lines), inner);
}

// --- proton -------------------------------------------------------------------

fn draw_proton(f: &mut Frame, app: &App, area: Rect) {
    let task_h = if app.task.is_some() { 3 } else { 0 };
    let [head, task_area, list_area] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(task_h),
        Constraint::Min(3),
    ])
    .areas(area);

    let default = &app.config.proton.default;
    let resolved = app.resolved(default);
    let block = pane("Default", false);
    let inner = block.inner(head);
    f.render_widget(block, head);
    let row = |label: &str, value: Span<'static>| {
        Line::from(vec![
            Span::styled(format!(" {label:<8} "), theme::dim()),
            value,
        ])
    };
    let w = inner.width.saturating_sub(11) as usize;
    let lines = vec![
        row(
            "Proton",
            Span::styled(
                app.describe_proton(default),
                if resolved.is_some() {
                    theme::bold()
                } else {
                    theme::err()
                },
            ),
        ),
        row(
            "Prefer",
            Span::raw(format!(
                "{} builds when `dwproton` matches several",
                app.config.proton.prefer
            )),
        ),
        row(
            "Path",
            match resolved {
                Some(p) => Span::styled(truncate_left(&p.display().to_string(), w), theme::dim()),
                None => Span::styled("not found — press i to install dwproton", theme::warn()),
            },
        ),
    ];
    f.render_widget(Paragraph::new(lines), inner);

    if let Some(task) = &app.task {
        let block = pane("Installing dwproton", true);
        let inner = block.inner(task_area);
        f.render_widget(block, task_area);
        match task.ratio() {
            Some(r) => f.render_widget(
                LineGauge::default()
                    .filled_style(theme::accent())
                    .unfilled_style(theme::dim())
                    .line_set(ratatui::symbols::line::THICK)
                    .label(truncate_right(&gauge_label(task), inner.width as usize / 2))
                    .ratio(r),
                inner,
            ),
            None => f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {} ", spinner(app.tick)), theme::accent()),
                    Span::raw(truncate_right(&task.stage, inner.width as usize - 4)),
                ])),
                inner,
            ),
        }
    }

    let block = pane(
        format!("Installed {}", app.protons.len()),
        app.overlays.is_empty(),
    );
    let inner = block.inner(list_area);
    f.render_widget(block, list_area);
    if app.protons.is_empty() {
        centered_message(
            f,
            inner,
            vec![
                Line::from(Span::styled("No Proton found", theme::bold())),
                Line::from(vec![
                    Span::styled("i", theme::key()),
                    Span::styled(
                        " to install dwproton, or install Proton in Steam",
                        theme::dim(),
                    ),
                ]),
            ],
        );
        return;
    }
    let [rows_area, path_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let active = app.active_proton_index();
    let w = rows_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .protons
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let star = if Some(i) == active {
                Span::styled("★ ", theme::ok())
            } else {
                Span::raw("  ")
            };
            let users = app.proton_users(p);
            let right = vec![
                Span::styled(format!("{:<8}", p.source), theme::dim()),
                Span::styled(
                    match users {
                        0 => "        ".to_string(),
                        1 => "  1 app ".to_string(),
                        n => format!("{n:>3} apps"),
                    },
                    theme::dim(),
                ),
                Span::raw(" "),
            ];
            let rw: usize = right.iter().map(Span::width).sum();
            let name = truncate_right(&p.name, w.saturating_sub(rw + 3));
            ListItem::new(spread(vec![star, Span::raw(name)], right, w))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.proton_sel));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▌ ")
            .highlight_style(theme::selected()),
        rows_area,
        &mut state,
    );
    if let Some(p) = app.protons.get(app.proton_sel) {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", truncate_left(&p.path.display().to_string(), w)),
                theme::dim(),
            )),
            path_area,
        );
    }
}

// --- prefixes -----------------------------------------------------------------

fn draw_prefixes(f: &mut Frame, app: &App, area: Rect) {
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(6)]).areas(area);
    let block = pane(
        format!("Prefixes {}", app.prefixes.len()),
        app.overlays.is_empty(),
    );
    let inner = block.inner(list_area);
    f.render_widget(block, list_area);
    let w = inner.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = app
        .prefixes
        .iter()
        .map(|id| {
            let is_default = *id == app.config.default_prefix;
            let star = if is_default {
                Span::styled("★ ", theme::ok())
            } else {
                Span::raw("  ")
            };
            let n = app.prefix_users(id).len();
            let exists = app.paths.prefix_path(id).is_dir();
            let right = vec![
                if exists {
                    Span::raw("")
                } else {
                    Span::styled("missing  ", theme::warn())
                },
                Span::styled(
                    match n {
                        0 => "unused ".to_string(),
                        1 => " 1 app ".to_string(),
                        n => format!("{n:>2} apps"),
                    },
                    theme::dim(),
                ),
                Span::raw(" "),
            ];
            ListItem::new(spread(vec![star, Span::raw(id.clone())], right, w))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.prefix_sel));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▌ ")
            .highlight_style(theme::selected()),
        inner,
        &mut state,
    );

    let block = pane("Details", false);
    let inner = block.inner(detail_area);
    f.render_widget(block, detail_area);
    let Some(id) = app.prefixes.get(app.prefix_sel) else {
        return;
    };
    let w = inner.width.saturating_sub(12) as usize;
    let users = app.prefix_users(id);
    let lines = vec![
        Line::from(vec![
            Span::styled(" Path      ", theme::dim()),
            Span::raw(truncate_left(
                &app.paths.prefix_path(id).display().to_string(),
                w,
            )),
        ]),
        Line::from(vec![
            Span::styled(" Used by   ", theme::dim()),
            Span::raw(if users.is_empty() {
                "no apps".to_string()
            } else {
                truncate_right(&users.join(", "), w)
            }),
        ]),
        Line::from(vec![
            Span::styled(" Default   ", theme::dim()),
            if *id == app.config.default_prefix {
                Span::styled("★ new apps share this bottle", theme::ok())
            } else {
                Span::styled("press s to make new apps share this bottle", theme::dim())
            },
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

// --- settings -----------------------------------------------------------------

fn setting_value(app: &App, s: Setting) -> String {
    let l = &app.config.launch;
    match s {
        Setting::Locale => match app.config.locale.as_deref().and_then(locale::find) {
            Some(loc) => format!("{} · {}", loc.label, loc.posix),
            None => "not set".into(),
        },
        Setting::Graphics => config::normalize_graphics(&l.graphics).into(),
        Setting::Sanitize => if l.sanitize_vulkan { "on" } else { "off" }.into(),
        Setting::LogLevel => config::normalize_log_level(&l.log_level).into(),
        Setting::Gamescope => l.gamescope.status_label(),
    }
}

fn draw_settings(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.overlays.is_empty();
    let block = pane("Settings", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = 0;
    let row = |label: String, value: Span<'static>, selected: bool, cycles: bool| {
        let (open, close) = if selected && cycles {
            ("‹ ", " ›")
        } else {
            ("  ", "  ")
        };
        let line = Line::from(vec![
            Span::styled(format!(" {label:<22}"), theme::dim()),
            Span::styled(open, theme::accent()),
            value,
            Span::styled(close, theme::accent()),
        ]);
        if selected {
            spread(line.spans, vec![], w).style(theme::selected())
        } else {
            line
        }
    };

    lines.push(Line::from(Span::styled(
        " Launch defaults",
        theme::accent(),
    )));
    for (i, s) in Setting::ALL.iter().enumerate() {
        let selected = focused && app.settings_sel == i;
        if selected {
            sel_line = lines.len();
        }
        lines.push(row(
            s.label().to_string(),
            Span::raw(setting_value(app, *s)),
            selected,
            true,
        ));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(" Environment", theme::accent()),
        Span::styled("  [launch.env] — passed to every launch", theme::dim()),
    ]));
    let base = Setting::ALL.len();
    for (i, (k, v)) in app.config.launch.env.iter().enumerate() {
        let selected = focused && app.settings_sel == base + i;
        if selected {
            sel_line = lines.len();
        }
        let value = truncate_right(v, w.saturating_sub(30));
        lines.push(row(k.clone(), Span::raw(value), selected, false));
    }
    let add_sel = focused && app.settings_sel == app.settings_len() - 1;
    if add_sel {
        sel_line = lines.len();
    }
    let add = Line::from(Span::styled(" + Add variable", theme::dim()));
    lines.push(if add_sel {
        spread(add.spans, vec![], w).style(theme::selected())
    } else {
        add
    });

    let h = inner.height as usize;
    let scroll = (sel_line + 1).saturating_sub(h) as u16;
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

// --- doctor -------------------------------------------------------------------

fn draw_doctor(f: &mut Frame, app: &App, area: Rect) {
    let block = pane("Health check", app.overlays.is_empty());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width.saturating_sub(4) as usize;

    let mut lines: Vec<Line> = Vec::new();
    for c in &app.checks {
        let title_style = match c.level {
            Level::Err => theme::err(),
            Level::Warn => theme::warn(),
            _ => theme::text(),
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            level_icon(c.level),
            Span::raw(" "),
            Span::styled(c.title.clone(), title_style),
        ]));
        if !c.detail.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("   {}", truncate_left(&c.detail, w)),
                theme::dim(),
            )));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(" Paths", theme::accent())));
    let p = &app.paths;
    for (label, path) in [
        ("Config", &p.config_file),
        ("Apps", &p.apps_file),
        ("Prefixes", &p.prefixes_dir),
        ("Runtimes", &p.runtimes_dir),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("   {label:<10}"), theme::dim()),
            Span::raw(truncate_left(
                &path.display().to_string(),
                w.saturating_sub(10),
            )),
        ]));
    }

    let max = (lines.len() as u16).saturating_sub(inner.height);
    let scroll = app.doctor_scroll.min(max);
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_the_meaningful_end() {
        assert_eq!(truncate_left("/a/b/c.exe", 20), "/a/b/c.exe");
        assert_eq!(truncate_left("/very/long/path/game.exe", 10), "…/game.exe");
        assert_eq!(truncate_right("Elden Ring", 6), "Elden…");
        assert_eq!(width(&truncate_right("原神原神原神", 5)), 5);
    }
}
