//! Styles. Body text keeps the terminal's default foreground so light and dark
//! themes both stay readable; never use `Color::White` for plain text.
//! Highlights use fg+bg pairs whose contrast does not depend on the theme.

use ratatui::style::{Color, Modifier, Style};

pub fn text() -> Style {
    Style::default()
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    // DarkGray stays secondary on dark themes and readable on light ones;
    // Modifier::DIM alone is unreliable across terminals.
    Style::default().fg(Color::DarkGray)
}

pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn brand() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::default().fg(Color::Green)
}

pub fn warn() -> Style {
    // Bold helps yellow survive light backgrounds.
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

pub fn err() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

/// Per-app value that overrides the global default.
pub fn overridden() -> Style {
    Style::default().fg(Color::Magenta)
}

/// Cursor row in the focused list.
pub fn selected() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// Cursor row in a list that does not have focus.
pub fn selected_idle() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn tab_active() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

pub fn tab_idle() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn border() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn border_focus() -> Style {
    Style::default().fg(Color::Cyan)
}

pub fn pane_title() -> Style {
    bold()
}

pub fn key() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn key_label() -> Style {
    Style::default().fg(Color::DarkGray)
}
