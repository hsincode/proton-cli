//! Single-line text input and fuzzy matching shared by every TUI form.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Span;

/// Editable single-line buffer with a cursor (char index, not byte index).
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn trimmed(&self) -> &str {
        self.value.trim()
    }

    pub fn is_empty(&self) -> bool {
        self.value.trim().is_empty()
    }

    pub fn set(&mut self, value: impl Into<String>) {
        *self = Self::new(value);
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Display width of the text left of the cursor (for terminal cursor placement).
    pub fn cursor_width(&self) -> u16 {
        let before: String = self.value.chars().take(self.cursor).collect();
        Span::raw(before).width() as u16
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    /// Apply an editing key. Returns `true` when the key was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Esc followed quickly by a key arrives as Alt+key; never type those.
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        match key.code {
            KeyCode::Char('u') if ctrl => {
                let b = self.byte_at(self.cursor);
                self.value.replace_range(..b, "");
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.len(),
            KeyCode::Char(c) if !ctrl && !c.is_control() => {
                let b = self.byte_at(self.cursor);
                self.value.insert(b, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let b = self.byte_at(self.cursor);
                    self.value.remove(b);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.len() {
                    let b = self.byte_at(self.cursor);
                    self.value.remove(b);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }

    fn delete_word(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
        self.value.replace_range(a..b, "");
        self.cursor = start;
    }
}

/// Case-insensitive subsequence match. Returns a score (lower is better) or
/// `None` when `query` does not match. Contiguous and early matches rank first.
pub fn fuzzy_score(query: &str, target: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let target = target.to_lowercase();
    if let Some(pos) = target.find(&query) {
        // Substring hits always beat scattered subsequence hits.
        return Some(pos);
    }
    let mut score = 1000;
    let mut last: Option<usize> = None;
    let mut chars = target.char_indices();
    for q in query.chars() {
        let (i, _) = chars.find(|(_, c)| *c == q)?;
        if let Some(prev) = last {
            score += i - prev - 1;
        } else {
            score += i;
        }
        last = Some(i);
    }
    Some(score)
}

/// Best score of `query` against any of `targets`.
pub fn fuzzy_best(query: &str, targets: &[&str]) -> Option<usize> {
    targets.iter().filter_map(|t| fuzzy_score(query, t)).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn edits_at_cursor_with_multibyte_text() {
        let mut t = TextInput::new("原神");
        t.handle_key(key(KeyCode::Left));
        t.handle_key(key(KeyCode::Char('x')));
        assert_eq!(t.value(), "原x神");
        t.handle_key(key(KeyCode::Backspace));
        assert_eq!(t.value(), "原神");
        assert_eq!(t.cursor_width(), 2);
    }

    #[test]
    fn ctrl_w_and_ctrl_u() {
        let mut t = TextInput::new("elden ring");
        t.handle_key(ctrl('w'));
        assert_eq!(t.value(), "elden ");
        t.handle_key(ctrl('u'));
        assert_eq!(t.value(), "");
    }

    #[test]
    fn ctrl_and_alt_keys_do_not_insert_text() {
        let mut t = TextInput::default();
        assert!(!t.handle_key(ctrl('s')));
        let mut alt_q = key(KeyCode::Char('q'));
        alt_q.modifiers = KeyModifiers::ALT;
        assert!(!t.handle_key(alt_q));
        assert_eq!(t.value(), "");
    }

    #[test]
    fn fuzzy_prefers_substring_then_subsequence() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert_eq!(fuzzy_score("ring", "Elden Ring"), Some(6));
        assert!(fuzzy_score("er", "Elden Ring").is_some());
        assert!(fuzzy_score("ring", "Elden Ring") < fuzzy_score("er", "Elden Ring"));
        assert_eq!(fuzzy_score("zz", "Elden Ring"), None);
    }

    #[test]
    fn fuzzy_best_uses_any_target() {
        assert!(fuzzy_best("eldr", &["Elden Ring", "elden-ring"]).is_some());
        assert_eq!(fuzzy_best("xyz", &["a", "b"]), None);
    }
}
