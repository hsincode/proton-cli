//! Rendering of overlays (pickers, forms, dialogs, browser, palette, help).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::app::App;
use super::keys::filtered_commands;
use super::overlay::{
    proton_options, AddField, AddForm, AppEnv, BrowseItem, Browser, Confirm, EnvField, EnvForm,
    EnvScope, InputDialog, Overlay, Palette, Picker, PrefixMode,
};
use super::text::TextInput;
use super::theme;
use super::view::{spread, tab_key_hints as tab_hints, truncate_left, truncate_right, width};

pub fn draw(f: &mut Frame, app: &App, ov: &Overlay, top: bool) {
    match ov {
        Overlay::Picker(p) => draw_picker(f, p, top),
        Overlay::Input(d) => draw_input(f, d, top),
        Overlay::Confirm(c) => draw_confirm(f, c),
        Overlay::AddApp(form) => draw_add(f, app, form, top),
        Overlay::AppEnv(e) => draw_app_env(f, app, e, top),
        Overlay::EnvEdit(form) => draw_env_edit(f, form, top),
        Overlay::Browser(b) => draw_browser(f, b, top),
        Overlay::Palette(p) => draw_palette(f, app, p),
        Overlay::Help => draw_help(f, app),
    }
}

pub fn key_hints(ov: &Overlay) -> Vec<(&'static str, &'static str)> {
    match ov {
        Overlay::Picker(_) => vec![("↑↓", "choose"), ("⏎", "apply"), ("esc", "cancel")],
        Overlay::Input(_) => vec![("⏎", "save"), ("esc", "cancel")],
        Overlay::Confirm(_) => vec![("y", "yes"), ("n", "no")],
        Overlay::AddApp(f) => {
            let mut h = vec![("tab", "next field")];
            match f.field {
                AddField::Exe => h.push(("⏎", "browse")),
                AddField::Prefix | AddField::Proton => h.push(("←→", "change")),
                AddField::Save => h.push(("⏎", "add")),
                _ => {}
            }
            h.extend([("^s", "add"), ("esc", "cancel")]);
            h
        }
        Overlay::AppEnv(_) => vec![
            ("⏎", "toggle / edit"),
            ("a", "add"),
            ("d", "delete"),
            ("esc", "close"),
        ],
        Overlay::EnvEdit(_) => vec![("tab", "key/value"), ("⏎", "save"), ("esc", "cancel")],
        Overlay::Browser(b) => {
            let mut h = vec![("⏎", "open / pick"), ("←", "up")];
            if b.floor.is_none() {
                h.push(("~", "home"));
            }
            h.push(("esc", "cancel"));
            h
        }
        Overlay::Palette(_) => vec![
            ("type", "search"),
            ("↑↓", "choose"),
            ("⏎", "run"),
            ("esc", "close"),
        ],
        Overlay::Help => vec![("any key", "close")],
    }
}

// --- frame helpers --------------------------------------------------------------

/// Centered box of at most `w`×`h`, cleared and framed.
fn modal(f: &mut Frame, title: &str, w: u16, h: u16, top: bool) -> Rect {
    let area = f.area();
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if top {
            theme::border_focus()
        } else {
            theme::border()
        })
        .title(Span::styled(format!(" {title} "), theme::pane_title()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    // One column of padding inside the frame.
    Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    )
}

/// One input row: `label  [value]`, with the terminal cursor when focused.
fn input_row(
    f: &mut Frame,
    area: Rect,
    label: &str,
    label_w: usize,
    input: &TextInput,
    placeholder: &str,
    focused: bool,
) {
    let value_x = area.x + label_w as u16 + 1;
    let max = area.width.saturating_sub(label_w as u16 + 2) as usize;
    let (text, style) = if input.value().is_empty() {
        (placeholder.to_string(), theme::dim())
    } else {
        (input.value().to_string(), theme::text())
    };
    let label_style = if focused {
        theme::accent()
    } else {
        theme::dim()
    };
    let line = Line::from(vec![
        Span::styled(format!("{label:<label_w$} "), label_style),
        Span::styled(truncate_left(&text, max), style),
    ]);
    f.render_widget(Paragraph::new(line), area);
    if focused {
        let x = (value_x + input.cursor_width()).min(area.x + area.width.saturating_sub(1));
        f.set_cursor_position((x, area.y));
    }
}

fn error_line(err: &Option<String>) -> Line<'static> {
    match err {
        Some(e) => Line::from(Span::styled(format!("✕ {e}"), theme::err())),
        None => Line::default(),
    }
}

fn rows(area: Rect, n: usize) -> Vec<Rect> {
    Layout::vertical(
        std::iter::repeat(Constraint::Length(1))
            .take(n)
            .chain([Constraint::Min(0)]),
    )
    .split(area)
    .iter()
    .take(n)
    .copied()
    .collect()
}

// --- simple overlays ------------------------------------------------------------

fn draw_picker(f: &mut Frame, p: &Picker, top: bool) {
    let content_w = p
        .items
        .iter()
        .map(|i| width(&i.label) + width(&i.hint) + 8)
        .max()
        .unwrap_or(20)
        .max(width(&p.title) + 6);
    let w = (content_w as u16 + 4).clamp(40, 90);
    let h = p.items.len() as u16 + 2;
    let inner = modal(f, &p.title, w, h, top);
    let iw = inner.width as usize;
    let items: Vec<ListItem> = p
        .items
        .iter()
        .map(|i| {
            let label_w = width(&i.label);
            let hint = truncate_left(&i.hint, iw.saturating_sub(label_w + 6));
            ListItem::new(spread(
                vec![Span::raw(i.label.clone())],
                vec![Span::styled(hint, theme::dim()), Span::raw(" ")],
                iw.saturating_sub(2),
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(p.sel));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▌ ")
            .highlight_style(theme::selected()),
        inner,
        &mut state,
    );
}

fn draw_input(f: &mut Frame, d: &InputDialog, top: bool) {
    let inner = modal(f, &d.title, 64, 5, top);
    let r = rows(inner, 3);
    input_row(f, r[0], &d.label, 6, &d.input, "", top);
    f.render_widget(Paragraph::new(error_line(&d.error)), r[2]);
}

fn draw_confirm(f: &mut Frame, c: &Confirm) {
    let w = c.lines.iter().map(|l| width(l)).max().unwrap_or(30).max(36) as u16 + 6;
    let inner = modal(f, &c.title, w, c.lines.len() as u16 + 4, true);
    let mut lines: Vec<Line> = c.lines.iter().map(|l| Line::raw(l.clone())).collect();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme::key()),
        Span::styled(" yes    ", theme::dim()),
        Span::styled("n", theme::key()),
        Span::styled(" no", theme::dim()),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}

// --- add app ----------------------------------------------------------------------

fn draw_add(f: &mut Frame, app: &App, form: &AddForm, top: bool) {
    let inner = modal(f, "Add app", 80, 13, top);
    let r = rows(inner, 10);
    const LW: usize = 8;
    let focused = |field: AddField| top && form.field == field;

    input_row(
        f,
        r[0],
        "Exe",
        LW,
        &form.exe,
        "⏎ to browse, or type a path to a .exe",
        focused(AddField::Exe),
    );
    let auto_name = if form.exe.is_empty() {
        "from the exe file name".to_string()
    } else {
        form.resolved_name()
    };
    input_row(
        f,
        r[1],
        "Name",
        LW,
        &form.name,
        &auto_name,
        focused(AddField::Name),
    );
    let auto_id = if form.exe.is_empty() && form.name.is_empty() {
        "from the name".to_string()
    } else {
        form.resolved_id()
    };
    input_row(f, r[2], "Id", LW, &form.id, &auto_id, focused(AddField::Id));

    // Prefix: mode selector, plus a text box in custom mode.
    let prefix_focus = focused(AddField::Prefix);
    let mode = match form.prefix_mode {
        PrefixMode::Default => format!("shared `{}`", app.config.default_prefix),
        PrefixMode::Auto => "own bottle (named after the id)".into(),
        PrefixMode::Custom => "custom id".into(),
    };
    let chooser = |label: &str, value: String, focus: bool| {
        Line::from(vec![
            Span::styled(
                format!("{label:<LW$} "),
                if focus { theme::accent() } else { theme::dim() },
            ),
            Span::styled(if focus { "‹ " } else { "  " }, theme::accent()),
            Span::styled(value, if focus { theme::bold() } else { theme::text() }),
            Span::styled(if focus { " ›" } else { "  " }, theme::accent()),
        ])
    };
    f.render_widget(Paragraph::new(chooser("Prefix", mode, prefix_focus)), r[3]);
    if form.prefix_mode == PrefixMode::Custom {
        input_row(
            f,
            r[4],
            "",
            LW,
            &form.prefix_custom,
            "type an id, e.g. my-bottle",
            prefix_focus,
        );
    } else {
        let path = form
            .resolved_prefix(&app.config.default_prefix)
            .map(|p| app.paths.prefix_path(&p).display().to_string())
            .unwrap_or_default();
        let max = inner.width as usize - LW - 3;
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" ".repeat(LW + 3)),
                Span::styled(truncate_left(&path, max), theme::dim()),
            ])),
            r[4],
        );
    }

    let proton = match proton_options(&app.protons)
        .get(form.proton_sel)
        .cloned()
        .flatten()
    {
        Some(name) => name,
        None => format!(
            "global default ({})",
            app.describe_proton(&app.config.proton.default)
        ),
    };
    f.render_widget(
        Paragraph::new(chooser("Proton", proton, focused(AddField::Proton))),
        r[5],
    );

    let save_focus = focused(AddField::Save);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" ".repeat(LW + 1)),
            Span::styled(
                "  Add app  ",
                if save_focus {
                    theme::selected()
                } else {
                    theme::accent()
                },
            ),
        ])),
        r[7],
    );
    f.render_widget(Paragraph::new(error_line(&form.error)), r[9]);
}

// --- env --------------------------------------------------------------------------

fn draw_app_env(f: &mut Frame, app: &App, e: &AppEnv, top: bool) {
    let Some(entry) = app.db.apps.get(&e.id) else {
        return;
    };
    let title = format!("Environment — {}", entry.name);
    let h = (entry.env.len() as u16 + 7).min(24);
    let inner = modal(f, &title, 76, h, top);
    let w = inner.width as usize;

    let mode = if entry.use_custom_env {
        "global + custom variables below"
    } else {
        "global variables only"
    };
    let mut lines = vec![
        {
            let l = Line::from(vec![
                Span::styled("Mode    ", theme::dim()),
                Span::raw(mode.to_string()),
            ]);
            if top && e.sel == 0 {
                spread(l.spans, vec![], w).style(theme::selected())
            } else {
                l
            }
        },
        Line::from(Span::styled(
            format!(
                "        {} global variable(s) from Settings always apply",
                app.config.launch.env.len()
            ),
            theme::dim(),
        )),
        Line::default(),
    ];
    if entry.env.is_empty() {
        lines.push(Line::from(Span::styled(
            "No custom variables — a to add one",
            theme::dim(),
        )));
    }
    for (i, (k, v)) in entry.env.iter().enumerate() {
        let style = if entry.use_custom_env {
            theme::text()
        } else {
            theme::dim()
        };
        let l = Line::from(vec![
            Span::styled(format!("{k} "), style),
            Span::styled("= ", theme::dim()),
            Span::styled(truncate_right(v, w.saturating_sub(width(k) + 4)), style),
        ]);
        lines.push(if top && e.sel == i + 1 {
            spread(l.spans, vec![], w).style(theme::selected())
        } else {
            l
        });
    }
    if !entry.use_custom_env && !entry.env.is_empty() {
        lines.push(Line::from(Span::styled(
            "(inactive until Mode is switched to custom)",
            theme::dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_env_edit(f: &mut Frame, form: &EnvForm, top: bool) {
    let title = match (&form.scope, &form.original) {
        (EnvScope::Global, None) => "New global variable",
        (EnvScope::Global, Some(_)) => "Edit global variable",
        (EnvScope::App(_), None) => "New app variable",
        (EnvScope::App(_), Some(_)) => "Edit app variable",
    };
    let inner = modal(f, title, 72, 6, top);
    let r = rows(inner, 4);
    input_row(
        f,
        r[0],
        "Key",
        6,
        &form.key,
        "e.g. DXVK_HUD",
        top && form.field == EnvField::Key,
    );
    input_row(
        f,
        r[1],
        "Value",
        6,
        &form.value,
        "",
        top && form.field == EnvField::Value,
    );
    f.render_widget(Paragraph::new(error_line(&form.error)), r[3]);
}

// --- browser ----------------------------------------------------------------------

fn draw_browser(f: &mut Frame, b: &Browser, top: bool) {
    let area = f.area();
    let title = if b.floor.is_some() {
        "Pick exe — wineprefix"
    } else {
        "Pick exe"
    };
    let inner = modal(
        f,
        title,
        (area.width * 4 / 5).max(60),
        (area.height * 4 / 5).max(12),
        top,
    );
    let [head, list_area, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let w = inner.width as usize;
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                truncate_left(&b.cwd.display().to_string(), w),
                theme::accent(),
            )),
            Line::from(Span::styled("Showing folders and .exe files", theme::dim())),
        ]),
        head,
    );

    if b.len() == 0 {
        f.render_widget(
            Paragraph::new(Span::styled("Nothing here", theme::dim())),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = (0..b.len())
            .filter_map(|i| b.item(i))
            .map(|item| match item {
                BrowseItem::Parent => ListItem::new(Span::styled("../", theme::dim())),
                BrowseItem::Entry(e) if e.is_dir => {
                    ListItem::new(Span::styled(format!("{}/", e.name), theme::accent()))
                }
                BrowseItem::Entry(e) => ListItem::new(Span::styled(e.name.clone(), theme::bold())),
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(b.sel));
        f.render_stateful_widget(
            List::new(items)
                .highlight_symbol("▌ ")
                .highlight_style(theme::selected()),
            list_area,
            &mut state,
        );
    }

    let foot_line = match &b.error {
        Some(e) => Line::from(Span::styled(
            format!("✕ {}", truncate_right(e, w - 2)),
            theme::err(),
        )),
        None if b.len() > 0 => Line::from(Span::styled(
            format!("{}/{}", b.sel + 1, b.len()),
            theme::dim(),
        )),
        None => Line::default(),
    };
    f.render_widget(Paragraph::new(foot_line), foot);
}

// --- palette / help ---------------------------------------------------------------

fn draw_palette(f: &mut Frame, app: &App, p: &Palette) {
    let matches = filtered_commands(app, p.input.value());
    let area = f.area();
    let w = 72.min(area.width.saturating_sub(4));
    let h = (matches.len() as u16 + 4)
        .clamp(6, 20)
        .min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + 2.min(area.height / 4),
        w,
        h,
    );
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focus())
        .title(Span::styled(" Commands ", theme::pane_title()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let [input_area, sep, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", theme::accent()),
            if p.input.value().is_empty() {
                Span::styled("type to search apps and commands", theme::dim())
            } else {
                Span::raw(p.input.value().to_string())
            },
        ])),
        input_area,
    );
    f.set_cursor_position((input_area.x + 2 + p.input.cursor_width(), input_area.y));
    f.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(sep.width as usize),
            theme::border(),
        )),
        sep,
    );

    if matches.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No matches", theme::dim())),
            list_area,
        );
        return;
    }
    let lw = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            let hint_w = width(&c.hint).min(lw / 3);
            ListItem::new(spread(
                vec![Span::raw(truncate_right(
                    &c.label,
                    lw.saturating_sub(hint_w + 2),
                ))],
                vec![
                    Span::styled(truncate_right(&c.hint, hint_w), theme::dim()),
                    Span::raw(" "),
                ],
                lw,
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(p.sel.min(matches.len() - 1)));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▌ ")
            .highlight_style(theme::selected()),
        list_area,
        &mut state,
    );
}

fn draw_help(f: &mut Frame, app: &App) {
    let section = |title: &str| Line::from(Span::styled(title.to_string(), theme::accent()));
    let row = |k: &str, label: &str| {
        Line::from(vec![
            Span::styled(format!("  {k:<12}"), theme::key()),
            Span::raw(label.to_string()),
        ])
    };
    let mut lines = vec![
        section("Everywhere"),
        row("1-5 / tab", "switch tab"),
        row(":", "command palette (apps and actions)"),
        row("↑↓ j k", "move   (also ^n ^p, g / G for top / bottom)"),
        row("?", "this help"),
        row("q  ^c", "quit"),
        Line::default(),
        section(app.tab.title()),
    ];
    let skip = ["tab", ":", "?", "q"];
    for (k, label) in tab_hints(app) {
        if !skip.contains(&k) {
            lines.push(row(k, label));
        }
    }
    let h = lines.len() as u16 + 2;
    let inner = modal(f, "Keyboard shortcuts", 64, h, true);
    f.render_widget(Paragraph::new(lines), inner);
}
