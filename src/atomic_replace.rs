//! Fit-for-purpose literal replace across one marked document workset.
//!
//! The component follows marked-set/query-replace precedent while preserving
//! Vellum's stronger contract: preview binds exact revisions and confirmation
//! submits one owner-atomic in-memory transaction. Persistence remains separate.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use vellum_sdk_client::{
    protocol::{ServiceAccessErrorV1, ServicePort},
    workspace::WorkspaceClientErrorV1,
};

use crate::{AtomicReplaceCommitV1, AtomicReplacePreviewV1, ServiceEditor, ServiceEditorError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtomicReplaceField {
    Query,
    Replacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtomicReplaceInput {
    Character(char),
    Backspace,
    NextField,
    Preview,
    Arm,
    Confirm,
    Recover,
    Back,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtomicReplaceLifecycle {
    Running,
    CloseRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtomicReplaceDisposition {
    Editing,
    Preview,
    Armed,
    Applied,
    Refused,
    OutcomeUnknown,
}

pub struct AtomicReplaceComponent {
    query: String,
    replacement: String,
    field: AtomicReplaceField,
    preview: Option<AtomicReplacePreviewV1>,
    armed: bool,
    commit: Option<AtomicReplaceCommitV1>,
    disposition: AtomicReplaceDisposition,
    notice: String,
}

impl Default for AtomicReplaceComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicReplaceComponent {
    pub const MAX_FIELD_BYTES: usize = 256;

    pub fn new() -> Self {
        Self {
            query: String::new(),
            replacement: String::new(),
            field: AtomicReplaceField::Query,
            preview: None,
            armed: false,
            commit: None,
            disposition: AtomicReplaceDisposition::Editing,
            notice: "Enter a literal query and replacement; Tab changes field".into(),
        }
    }

    pub fn update<T: ServicePort>(
        &mut self,
        input: AtomicReplaceInput,
        editor: &mut ServiceEditor<T>,
        focused: bool,
    ) -> AtomicReplaceLifecycle {
        if !focused {
            return AtomicReplaceLifecycle::Running;
        }
        if input == AtomicReplaceInput::Close {
            if self.disposition == AtomicReplaceDisposition::OutcomeUnknown {
                self.notice =
                    "Exact atomic edit outcome is unknown; r replays only the retained request"
                        .into();
                return AtomicReplaceLifecycle::Running;
            }
            return AtomicReplaceLifecycle::CloseRequested;
        }
        match input {
            AtomicReplaceInput::Character(character)
                if self.disposition == AtomicReplaceDisposition::Editing =>
            {
                let target = self.active_field_mut();
                if target.len().saturating_add(character.len_utf8()) <= Self::MAX_FIELD_BYTES {
                    target.push(character);
                } else {
                    self.notice = "Atomic replace fields are bounded to 256 UTF-8 bytes".into();
                }
            }
            AtomicReplaceInput::Backspace
                if self.disposition == AtomicReplaceDisposition::Editing =>
            {
                self.active_field_mut().pop();
            }
            AtomicReplaceInput::NextField
                if self.disposition == AtomicReplaceDisposition::Editing =>
            {
                self.field = match self.field {
                    AtomicReplaceField::Query => AtomicReplaceField::Replacement,
                    AtomicReplaceField::Replacement => AtomicReplaceField::Query,
                };
            }
            AtomicReplaceInput::Preview
                if self.disposition == AtomicReplaceDisposition::Editing =>
            {
                match editor.prepare_atomic_replace(&self.query, &self.replacement) {
                    Ok(preview) => {
                        self.notice = format!(
                            "Exact preview: {} matches across {} marked documents",
                            preview.total_matches,
                            preview.documents.len()
                        );
                        self.preview = Some(preview);
                        self.disposition = AtomicReplaceDisposition::Preview;
                    }
                    Err(error) => {
                        self.notice = format!("Atomic replace preview refused: {error}");
                        self.disposition = AtomicReplaceDisposition::Refused;
                    }
                }
            }
            AtomicReplaceInput::Arm if self.disposition == AtomicReplaceDisposition::Preview => {
                self.armed = true;
                self.disposition = AtomicReplaceDisposition::Armed;
                self.notice =
                    "ARMED — Enter publishes one owner-atomic in-memory transaction".into();
            }
            AtomicReplaceInput::Confirm if self.disposition == AtomicReplaceDisposition::Armed => {
                self.submit(editor);
            }
            AtomicReplaceInput::Recover
                if self.disposition == AtomicReplaceDisposition::OutcomeUnknown =>
            {
                self.notice = "Replaying the exact retained atomic edit request".into();
                self.submit(editor);
            }
            AtomicReplaceInput::Back => match self.disposition {
                AtomicReplaceDisposition::Armed => {
                    self.armed = false;
                    self.disposition = AtomicReplaceDisposition::Preview;
                    self.notice = "Atomic replace disarmed; exact preview retained".into();
                }
                AtomicReplaceDisposition::Preview
                | AtomicReplaceDisposition::Refused
                | AtomicReplaceDisposition::Applied => {
                    self.preview = None;
                    self.commit = None;
                    self.armed = false;
                    self.disposition = AtomicReplaceDisposition::Editing;
                    self.notice = "Edit query/replacement; Enter rebuilds exact preview".into();
                }
                _ => {}
            },
            AtomicReplaceInput::Character(_)
            | AtomicReplaceInput::Backspace
            | AtomicReplaceInput::NextField
            | AtomicReplaceInput::Preview
            | AtomicReplaceInput::Arm
            | AtomicReplaceInput::Confirm
            | AtomicReplaceInput::Recover
            | AtomicReplaceInput::Close => {}
        }
        AtomicReplaceLifecycle::Running
    }

    fn submit<T: ServicePort>(&mut self, editor: &mut ServiceEditor<T>) {
        let Some(preview) = self.preview.as_ref() else {
            self.disposition = AtomicReplaceDisposition::Refused;
            self.notice = "Atomic replace has no exact preview".into();
            return;
        };
        self.armed = false;
        match editor.commit_atomic_replace(preview) {
            Ok(commit) => {
                self.commit = Some(commit);
                self.disposition = AtomicReplaceDisposition::Applied;
                self.notice = format!(
                    "Owner generation {} published atomically across {} changed documents; persistence remains dirty",
                    commit.generation, commit.changed_documents
                );
            }
            Err(ServiceEditorError(WorkspaceClientErrorV1::Access(
                ServiceAccessErrorV1::OutcomeUnknown,
            ))) => {
                self.disposition = AtomicReplaceDisposition::OutcomeUnknown;
                self.notice =
                    "Atomic edit delivery is unknown; r replays the exact retained request".into();
            }
            Err(error) => {
                self.disposition = AtomicReplaceDisposition::Refused;
                self.notice = format!("Atomic edit refused: {error}; rebuild the exact preview");
            }
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            AtomicReplaceField::Query => &mut self.query,
            AtomicReplaceField::Replacement => &mut self.replacement,
        }
    }

    pub fn draw_in(&self, frame: &mut Frame<'_>, area: Rect, focused: bool) {
        let [body, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let title = if focused {
            " Vellum atomic replace • focused "
        } else {
            " Vellum atomic replace "
        };
        let query_marker = if self.field == AtomicReplaceField::Query {
            ">"
        } else {
            " "
        };
        let replacement_marker = if self.field == AtomicReplaceField::Replacement {
            ">"
        } else {
            " "
        };
        let mut lines = vec![
            Line::from("One revision-bound in-memory transaction; saving remains separate."),
            Line::from(format!(
                "{query_marker} Find: {}",
                display_literal(&self.query)
            )),
            Line::from(format!(
                "{replacement_marker} Replace: {}",
                display_literal(&self.replacement)
            )),
            Line::from(""),
        ];
        if let Some(preview) = &self.preview {
            lines.push(Line::from(format!(
                "{} matches across {} exact documents",
                preview.total_matches,
                preview.documents.len()
            )));
            for document in &preview.documents {
                lines.push(Line::from(format!(
                    "  document {} @ revision {}  {} matches",
                    document.document_ordinal, document.expected_revision, document.matches
                )));
            }
        }
        lines.extend([
            Line::from(""),
            Line::from(match self.disposition {
                AtomicReplaceDisposition::Editing => "Tab field · Enter preview · Esc close",
                AtomicReplaceDisposition::Preview => "c arm · Esc edit",
                AtomicReplaceDisposition::Armed => "ARMED — Enter commit once · Esc disarm",
                AtomicReplaceDisposition::OutcomeUnknown => "r exact replay/recovery · exit fenced",
                AtomicReplaceDisposition::Applied | AtomicReplaceDisposition::Refused => {
                    "Esc edit · q close when resolved"
                }
            }),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(title)),
            body,
        );
        frame.render_widget(Paragraph::new(self.notice.as_str()), footer);
    }

    pub const fn disposition(&self) -> AtomicReplaceDisposition {
        self.disposition
    }

    pub fn notice(&self) -> &str {
        &self.notice
    }

    pub fn preview(&self) -> Option<&AtomicReplacePreviewV1> {
        self.preview.as_ref()
    }

    pub const fn commit(&self) -> Option<AtomicReplaceCommitV1> {
        self.commit
    }
}

fn display_literal(value: &str) -> String {
    let mut displayed = String::new();
    for character in value.chars().take(80) {
        for escaped in character.escape_default() {
            displayed.push(escaped);
        }
    }
    if value.chars().count() > 80 {
        displayed.push('…');
    }
    format!("\"{displayed}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_literals_escape_terminal_controls_and_are_bounded() {
        let displayed = display_literal("a\nb\u{1b}[31m");
        assert_eq!(displayed, "\"a\\nb\\u{1b}[31m\"");
        assert!(!displayed.contains('\n'));
    }
}
