use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Paragraph},
};
use vellum_adapter_clipboard_macos::Operation;
use vellum_model::{Action, DestinationPlan, EditorFrame, Movement};

pub enum Input {
    Clipboard(Option<Operation>),
    Edit(Action),
    Quit,
    CancelQuit,
    Open,
    Find,
    Replace(bool),
    Match(bool),
    SelectMatches,
    Destination(bool),
    Comment(bool),
    Comments,
    Ignore,
}
/// Bindings live only in this frontend. Release events and unsupported modifier
/// chords never become authored text. Terminal paste is handled as inert text.
pub fn translate_key(key: KeyEvent) -> Input {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Input::Ignore;
    }
    if key.modifiers == KeyModifiers::CONTROL {
        if matches!(key.code, KeyCode::Char('q' | 'c' | 'x' | 'v'))
            && key.kind != KeyEventKind::Press
        {
            return Input::Ignore;
        }
        return match key.code {
            KeyCode::Char('q') => Input::Quit,
            KeyCode::Char('c') => Input::Clipboard(Some(Operation::Copy)),
            KeyCode::Char('x') => Input::Clipboard(Some(Operation::Cut)),
            KeyCode::Char('v') => Input::Clipboard(Some(Operation::Paste)),
            KeyCode::Char('s') => Input::Edit(Action::Save),
            KeyCode::Char('z') => Input::Edit(Action::Undo),
            KeyCode::Char('y') => Input::Edit(Action::Redo),
            KeyCode::Char('a') => Input::Edit(Action::SelectAll),
            KeyCode::Char('o') if key.kind == KeyEventKind::Press => Input::Open,
            KeyCode::Char('f') if key.kind == KeyEventKind::Press => Input::Find,
            KeyCode::Char('r') if key.kind == KeyEventKind::Press => Input::Replace(false),
            _ => Input::Ignore,
        };
    }
    if key.modifiers.intersects(
        KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SUPER
            | KeyModifiers::HYPER
            | KeyModifiers::META,
    ) {
        return Input::Ignore;
    }
    if key.modifiers == KeyModifiers::SHIFT {
        let movement = match key.code {
            KeyCode::Left => Some(Movement::Left),
            KeyCode::Right => Some(Movement::Right),
            KeyCode::Up => Some(Movement::Up),
            KeyCode::Down => Some(Movement::Down),
            KeyCode::Home => Some(Movement::Home),
            KeyCode::End => Some(Movement::End),
            _ => None,
        };
        if let Some(movement) = movement {
            return Input::Edit(Action::ExtendSelection(movement));
        }
    }
    match key.code {
        KeyCode::Char(ch) => Input::Edit(Action::Insert(ch.to_string())),
        KeyCode::Enter => Input::Edit(Action::Insert("\n".into())),
        KeyCode::Tab => Input::Edit(Action::Insert("\t".into())),
        KeyCode::BackTab => Input::Edit(Action::Outdent),
        KeyCode::Backspace => Input::Edit(Action::Backspace),
        KeyCode::Delete => Input::Edit(Action::Delete),
        KeyCode::Left => Input::Edit(Action::Left),
        KeyCode::Right => Input::Edit(Action::Right),
        KeyCode::Up => Input::Edit(Action::Up),
        KeyCode::Down => Input::Edit(Action::Down),
        KeyCode::Home => Input::Edit(Action::Home),
        KeyCode::End => Input::Edit(Action::End),
        KeyCode::F(6) => Input::Edit(Action::NextDocument),
        KeyCode::F(5) => Input::Edit(Action::Indent),
        KeyCode::F(9) if key.kind == KeyEventKind::Press => Input::SelectMatches,
        KeyCode::F(10) if key.kind == KeyEventKind::Press => {
            Input::Comment(key.modifiers.contains(KeyModifiers::SHIFT))
        }
        KeyCode::F(11) if key.kind == KeyEventKind::Press => Input::Comments,
        KeyCode::F(12) if key.kind == KeyEventKind::Press => Input::Clipboard(None),
        KeyCode::F(2) if key.kind == KeyEventKind::Press => Input::Destination(true),
        KeyCode::F(7) if key.kind == KeyEventKind::Press => Input::Destination(false),
        KeyCode::F(3) => Input::Match(key.modifiers.contains(KeyModifiers::SHIFT)),
        KeyCode::F(8) if key.kind == KeyEventKind::Press => Input::Replace(true),
        KeyCode::F(4) => Input::Edit(Action::ToggleEditingLock),
        KeyCode::Esc => Input::CancelQuit,
        _ => Input::Ignore,
    }
}

/// Literal escaped target tail with an explicit truncation marker. Never render
/// raw control/bidi characters as part of an overwrite confirmation.
pub fn destination_notice(plan: &DestinationPlan, width: u16) -> String {
    let prefix = format!(
        "Y overwrite / Esc cancel [{}]: ",
        if plan.is_save_as() { "As" } else { "Copy" }
    );
    let width = usize::from(width);
    if width <= prefix.len() {
        return prefix[..width].into();
    }
    let escaped: String = plan
        .destination()
        .to_string_lossy()
        .chars()
        .flat_map(char::escape_default)
        .collect();
    let available = width - prefix.len();
    if escaped.len() <= available {
        return format!("{prefix}{escaped}");
    }
    let dots = available.min(3);
    let tail = available - dots;
    format!(
        "{prefix}{}{}",
        ".".repeat(dots),
        &escaped[escaped.len() - tail..]
    )
}

pub fn draw(frame: &mut Frame<'_>, view: &EditorFrame, notice: Option<&str>) {
    let [body, status, help] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let title = format!(
        "Vellum | document {}/{}{}{} | {} selection(s)",
        view.active + 1,
        view.documents,
        if view.dirty { " *" } else { "" },
        if view.editing_locked {
            " [editing locked]"
        } else {
            ""
        },
        view.selection_count,
    );
    frame.render_widget(
        Paragraph::new(view.lines.join("\n")).block(Block::bordered().title(title)),
        body,
    );
    let inner = body.inner(ratatui::layout::Margin::new(1, 1));
    for (row, selection) in view
        .selection_cells
        .iter()
        .take(usize::from(inner.height))
        .enumerate()
    {
        for selection in selection {
            for column in selection.start.min(inner.width)..selection.end.min(inner.width) {
                frame.buffer_mut()[(inner.x + column, inner.y + row as u16)]
                    .set_style(Style::default().fg(Color::White).bg(Color::Blue));
            }
        }
    }
    frame.render_widget(Paragraph::new(notice.unwrap_or(&view.status)), status);
    frame.render_widget(
        Paragraph::new(
            "Ctrl-S source | Ctrl-C/X/V clipboard | Ctrl-O/F | F10/F11 notes | Ctrl-Q quit",
        ),
        help,
    );
    if let Some((x, y)) = view.cursor {
        let inner = body.inner(ratatui::layout::Margin::new(1, 1));
        if x < inner.width && y < inner.height {
            frame.set_cursor_position((inner.x + x, inner.y + y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn keymaps_never_translate_release_or_control_chords_to_source_text() {
        for (key, expected) in [
            ('c', Operation::Copy),
            ('x', Operation::Cut),
            ('v', Operation::Paste),
        ] {
            assert!(
                matches!(translate_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL)), Input::Clipboard(Some(operation)) if operation == expected)
            );
        }
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)),
            Input::Comment(false)
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::SHIFT)),
            Input::Comment(true)
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::F(11), KeyModifiers::NONE)),
            Input::Comments
        ));
        let mut key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert!(matches!(translate_key(key), Input::Ignore));
        let mut repeat_quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        repeat_quit.kind = KeyEventKind::Repeat;
        assert!(matches!(translate_key(repeat_quit), Input::Ignore));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Input::Edit(Action::Save)
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Input::Quit
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            Input::Ignore
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)),
            Input::Edit(Action::NextDocument)
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)),
            Input::Edit(Action::ExtendSelection(Movement::Right))
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Input::Edit(Action::SelectAll)
        ));
        assert!(matches!(
            translate_key(KeyEvent::new(
                KeyCode::Right,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Input::Ignore
        ));
    }
    #[test]
    fn test_backend_renders_source_status_discard_prompt_and_tiny_windows() {
        let mut terminal = Terminal::new(TestBackend::new(72, 8)).unwrap();
        let view = EditorFrame {
            lines: vec!["    prose 界".into(), "```mermaid".into()],
            selection_cells: vec![vec![4..9], vec![]],
            selection_count: 1,
            cursor: Some((4, 0)),
            status: "Ready".into(),
            active: 0,
            documents: 2,
            dirty: true,
            editing_locked: false,
        };
        terminal
            .draw(|f| {
                draw(
                    f,
                    &view,
                    Some("Unsaved: Ctrl-Q again discards; Esc cancels"),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(rendered.contains("document 1/2 *"));
        assert!(rendered.contains("prose"));
        assert!(rendered.contains("mermaid"));
        assert!(rendered.contains("Esc cancels"));
        assert_eq!(buffer[(5, 1)].bg, Color::Blue);
        assert_ne!(buffer[(4, 1)].bg, Color::Blue);
        terminal.backend_mut().resize(1, 1);
        terminal.autoresize().unwrap();
        terminal.draw(|f| draw(f, &view, None)).unwrap();
    }

    #[test]
    fn actual_key_path_edits_history_and_saves_through_the_shared_host() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        use vellum_model::Editor;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = loop {
            let path = std::env::temp_dir().join(format!(
                "vellum-tui-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture directory: {e}"),
            }
        };
        let path = directory.join("source");
        fs::write(&path, b"A\r\n").unwrap();
        let mut editor = Editor::open(
            std::slice::from_ref(&path),
            crate::reference_session_config(),
        )
        .unwrap();
        editor
            .enable_native_unicode(
                vellum_adapter_unicode_icu::IcuFactory::reference(),
                vellum_model::NativeUnicodeProfile::reference(),
            )
            .unwrap();
        for key in [
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('ß'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        ] {
            let Input::Edit(action) = translate_key(key) else {
                panic!("expected edit action")
            };
            editor.apply(action).unwrap();
        }
        let view = editor.frame(70, 4).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(72, 8)).unwrap();
        terminal.draw(|frame| draw(frame, &view, None)).unwrap();
        assert!(!view.dirty);
        assert_eq!(fs::read(&path).unwrap(), "Aß\r\n".as_bytes());
        let plan = editor
            .plan_destination(directory.join("long-directory-name/filename"), true)
            .unwrap();
        let confirmation = destination_notice(&plan, 60);
        assert!(confirmation.contains("..."));
        assert!(confirmation.ends_with("filename"));
        for width in 0..60 {
            assert!(destination_notice(&plan, width).len() <= usize::from(width));
        }
        editor.close(false).unwrap();
        assert!(editor.is_closed());
        drop(editor);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
