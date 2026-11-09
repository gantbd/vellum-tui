//! Frontend-only bounded UTF-8 entry. No shell expansion or source publication.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

pub enum PromptInput {
    Pending,
    Cancel,
    Submit(String),
}
#[derive(Debug)]
pub enum PromptError {
    Capacity,
    ControlCharacter,
}
impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Capacity => "Text entry exceeds the frontend limit",
            Self::ControlCharacter => "Single-line entry refuses control characters",
        })
    }
}
/// A bounded UI buffer, not a filesystem path-length guarantee. Non-UTF-8 paths
/// remain available through startup arguments; this prompt cannot represent them.
pub struct TextPrompt {
    text: String,
    limit: usize,
    label: &'static str,
    multiline: bool,
}
impl TextPrompt {
    pub fn new(limit: usize, label: &'static str) -> Self {
        Self {
            text: String::new(),
            limit,
            label,
            multiline: false,
        }
    }
    /// UI scratch buffer only. Existing CR/LF/control bytes remain literal;
    /// terminal display escapes them, Enter generates LF, Ctrl-S submits.
    pub fn multiline(
        limit: usize,
        label: &'static str,
        initial: &str,
    ) -> Result<Self, PromptError> {
        let mut prompt = Self::new(limit, label);
        prompt.multiline = true;
        prompt.insert(initial)?;
        Ok(prompt)
    }
    /// All-or-none input: a bad paste must not submit a partial different path.
    pub fn insert(&mut self, text: &str) -> Result<(), PromptError> {
        if !self.multiline && text.chars().any(char::is_control) {
            return Err(PromptError::ControlCharacter);
        }
        if self
            .text
            .len()
            .checked_add(text.len())
            .is_none_or(|n| n > self.limit)
        {
            return Err(PromptError::Capacity);
        }
        self.text
            .try_reserve(text.len())
            .map_err(|_| PromptError::Capacity)?;
        self.text.push_str(text);
        Ok(())
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<PromptInput, PromptError> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Ok(PromptInput::Pending);
        }
        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL
                && matches!(key.code, KeyCode::Char('c' | 'q')))
        {
            return Ok(PromptInput::Cancel);
        }
        if self.multiline
            && key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('s')
            && key.kind == KeyEventKind::Press
        {
            return Ok(PromptInput::Submit(self.text.clone()));
        }
        if key.modifiers.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::HYPER
                | KeyModifiers::META,
        ) {
            return Ok(PromptInput::Pending);
        }
        match key.code {
            KeyCode::Enter if self.multiline => {
                self.insert("\n")?;
                Ok(PromptInput::Pending)
            }
            KeyCode::Tab if self.multiline => {
                self.insert("\t")?;
                Ok(PromptInput::Pending)
            }
            KeyCode::Enter if key.kind == KeyEventKind::Press => {
                Ok(PromptInput::Submit(self.text.clone()))
            }
            KeyCode::Backspace => {
                if self.multiline {
                    if let Some((start, _)) = self.text.grapheme_indices(true).next_back() {
                        self.text.truncate(start);
                    }
                } else {
                    self.text.pop();
                }
                Ok(PromptInput::Pending)
            }
            KeyCode::Char(ch) => {
                self.insert(ch.encode_utf8(&mut [0; 4]))?;
                Ok(PromptInput::Pending)
            }
            _ => Ok(PromptInput::Pending),
        }
    }
    /// Escape controls and format characters, including bidi overrides, before
    /// terminal rendering. The literal path used for open remains unchanged.
    pub fn display(&self, width: usize) -> String {
        let label: String = self.label.chars().flat_map(char::escape_default).collect();
        let prefix = format!(
            "{label} {}: ",
            if self.multiline {
                "(Ctrl-S commit/Enter LF/Esc)"
            } else {
                "(Enter/Esc)"
            }
        );
        if width <= prefix.len() {
            return prefix[..width].into();
        }
        let escaped: String = self.text.chars().flat_map(char::escape_default).collect();
        let available = width - prefix.len();
        // escape_default produces ASCII: byte clipping cannot split UTF-8 or
        // render a raw control/bidi character. Keep the path's edited tail visible.
        if escaped.len() <= available {
            format!("{prefix}{escaped}")
        } else {
            let tail = available.saturating_sub(1);
            format!("{prefix}~{}", &escaped[escaped.len() - tail..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_comment_entry_preserves_literal_bytes_until_explicit_commit_and_escapes_display() {
        let initial = "Grüße\r\n\u{202e}\0";
        let mut p = TextPrompt::multiline(128, "Comment", initial).unwrap();
        p.insert("e\u{301}").unwrap();
        p.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
        p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        p.insert("$(inert)").unwrap();
        let PromptInput::Submit(body) = p
            .key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap()
        else {
            panic!("commit expected")
        };
        assert_eq!(body, format!("{initial}\n$(inert)"));
        assert!(!p.display(100).contains('\u{202e}'));
        assert!(!p.display(100).contains('\0'));
        for width in 0..100 {
            assert!(p.display(width).len() <= width);
        }
        assert!(p.insert(&"x".repeat(129)).is_err());
        let PromptInput::Submit(after) = p
            .key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap()
        else {
            panic!("commit expected")
        };
        assert_eq!(after, body);
        assert!(matches!(
            p.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .unwrap(),
            PromptInput::Cancel
        ));
    }
    #[test]
    fn literal_path_prompt_is_bounded_inert_and_does_not_expand_or_partially_paste() {
        let mut p = TextPrompt::new(32, "Open");
        p.insert("~/$(no command)/é").unwrap();
        assert!(p.insert("\nother").is_err());
        assert!(p.insert(&"x".repeat(33)).is_err());
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let PromptInput::Submit(path) = p.key(enter).unwrap() else {
            panic!("expected open")
        };
        assert_eq!(path, "~/$(no command)/é");
        let mut release = enter;
        release.kind = KeyEventKind::Release;
        assert!(matches!(p.key(release).unwrap(), PromptInput::Pending));
        assert!(matches!(
            p.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .unwrap(),
            PromptInput::Cancel
        ));
        let mut bidi = TextPrompt::new(16, "Open");
        bidi.insert("a\u{202e}b").unwrap();
        assert!(!bidi.display(80).contains('\u{202e}'));
        for width in 0..40 {
            assert!(bidi.display(width).len() <= width);
        }
        let mut long = TextPrompt::new(100, "Open");
        long.insert("a/very/long/directory/filename").unwrap();
        assert!(long.display(28).ends_with("filename"));
        assert!(
            matches!(TextPrompt::new(32, "Replace").key(enter).unwrap(), PromptInput::Submit(text) if text.is_empty())
        );
    }
}
