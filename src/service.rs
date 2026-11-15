use unicode_width::UnicodeWidthStr;
use vellum_editor::EditorFrame;
use vellum_sdk_client::{
    protocol::{ServicePort, WorkspaceDocumentIdV1},
    workspace::{
        PersistenceProjectionV1, ProjectionFreshnessV1, WorkspaceClientErrorV1, WorkspaceClientV1,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceAction {
    Insert(String),
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    NextDocument,
    Save,
    Refresh,
}

/// Closed service-mode interpretation of presentation input. Hosts may own
/// lifecycle and prompts while reusing the exact edit mapping without reaching
/// through the TUI facade into the editor model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceInput {
    Quit,
    CancelQuit,
    Open,
    Apply(ServiceAction),
    Unsupported,
}

pub fn translate_service_input(input: crate::Input) -> ServiceInput {
    match input {
        crate::Input::Quit => ServiceInput::Quit,
        crate::Input::CancelQuit => ServiceInput::CancelQuit,
        crate::Input::Open => ServiceInput::Open,
        crate::Input::Edit(action) => match action {
            vellum_editor::Action::Insert(text) => ServiceInput::Apply(ServiceAction::Insert(text)),
            vellum_editor::Action::Backspace => ServiceInput::Apply(ServiceAction::Backspace),
            vellum_editor::Action::Delete => ServiceInput::Apply(ServiceAction::Delete),
            vellum_editor::Action::Left => ServiceInput::Apply(ServiceAction::Left),
            vellum_editor::Action::Right => ServiceInput::Apply(ServiceAction::Right),
            vellum_editor::Action::Up => ServiceInput::Apply(ServiceAction::Up),
            vellum_editor::Action::Down => ServiceInput::Apply(ServiceAction::Down),
            vellum_editor::Action::Home => ServiceInput::Apply(ServiceAction::Home),
            vellum_editor::Action::End => ServiceInput::Apply(ServiceAction::End),
            vellum_editor::Action::Save => ServiceInput::Apply(ServiceAction::Save),
            vellum_editor::Action::NextDocument => ServiceInput::Apply(ServiceAction::NextDocument),
            _ => ServiceInput::Unsupported,
        },
        _ => ServiceInput::Unsupported,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceEditorError(pub WorkspaceClientErrorV1);

impl std::fmt::Display for ServiceEditorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "service workspace refused: {:?}", self.0)
    }
}

impl std::error::Error for ServiceEditorError {}

impl From<WorkspaceClientErrorV1> for ServiceEditorError {
    fn from(value: WorkspaceClientErrorV1) -> Self {
        Self(value)
    }
}

/// UI-local cursor over one service-owned snapshot. Source, persistence and
/// membership authority remain in the wrapped workspace client.
pub struct ServiceEditor<T> {
    workspace: WorkspaceClientV1<T>,
    cursor: usize,
}

impl<T: ServicePort> ServiceEditor<T> {
    pub fn open(mut workspace: WorkspaceClientV1<T>) -> Result<Self, ServiceEditorError> {
        workspace.refresh_inventory()?;
        let mut editor = Self {
            workspace,
            cursor: 0,
        };
        editor.ensure_selection()?;
        Ok(editor)
    }

    pub const fn workspace(&self) -> &WorkspaceClientV1<T> {
        &self.workspace
    }

    pub fn disconnect(
        self,
    ) -> Result<T, vellum_sdk_client::workspace::WorkspaceDisconnectFailureV1<T>> {
        self.workspace.disconnect()
    }

    pub fn apply(&mut self, action: ServiceAction) -> Result<(), ServiceEditorError> {
        match action {
            ServiceAction::Insert(inserted) => {
                let at = self.cursor_u64()?;
                let bytes = inserted.len();
                self.workspace.replace(at, at, inserted)?;
                self.cursor = self.cursor.saturating_add(bytes);
            }
            ServiceAction::Backspace => {
                let Some(start) = self.previous_boundary() else {
                    return Ok(());
                };
                self.workspace
                    .replace(to_u64(start)?, self.cursor_u64()?, String::new())?;
                self.cursor = start;
            }
            ServiceAction::Delete => {
                let Some(end) = self.next_boundary() else {
                    return Ok(());
                };
                self.workspace
                    .replace(self.cursor_u64()?, to_u64(end)?, String::new())?;
            }
            ServiceAction::Left => {
                if let Some(previous) = self.previous_boundary() {
                    self.cursor = previous;
                }
            }
            ServiceAction::Right => {
                if let Some(next) = self.next_boundary() {
                    self.cursor = next;
                }
            }
            ServiceAction::Home => self.cursor = self.current_line().0,
            ServiceAction::End => self.cursor = self.current_line().1,
            ServiceAction::Up => self.move_vertical(false),
            ServiceAction::Down => self.move_vertical(true),
            ServiceAction::NextDocument => self.next_document()?,
            ServiceAction::Save => self.workspace.save()?,
            ServiceAction::Refresh => self.refresh()?,
        }
        self.clamp_cursor();
        Ok(())
    }

    pub fn admit_file(&mut self, locator: String) -> Result<(), ServiceEditorError> {
        self.workspace.admit_file(locator)?;
        self.workspace.refresh_inventory()?;
        self.ensure_selection()
    }

    pub fn admit_directory(&mut self, locator: String) -> Result<(), ServiceEditorError> {
        self.workspace.admit_directory(locator)?;
        self.workspace.refresh_inventory()?;
        self.ensure_selection()
    }

    pub fn poll(&mut self) -> Result<(), ServiceEditorError> {
        self.workspace.poll_changes()?;
        if self.workspace.projection().inventory_freshness() == ProjectionFreshnessV1::Stale {
            self.workspace.refresh_inventory()?;
        }
        if self.workspace.projection().snapshot_freshness() == ProjectionFreshnessV1::Stale
            && self.workspace.projection().persistence() != PersistenceProjectionV1::Indeterminate
        {
            self.workspace.refresh_snapshot()?;
        }
        self.clamp_cursor();
        Ok(())
    }

    pub fn frame(&self, height: u16) -> Option<EditorFrame> {
        let projection = self.workspace.projection();
        let snapshot = projection.snapshot()?;
        let text = snapshot.text.as_ref();
        let (line, column) = line_and_column(text, self.cursor);
        let visible = usize::from(height.max(1));
        let first = line.saturating_sub(visible.saturating_sub(1));
        let lines: Vec<String> = text
            .split('\n')
            .skip(first)
            .take(visible)
            .map(str::to_owned)
            .collect();
        let inventory = projection.inventory()?;
        let active = projection
            .selected()
            .and_then(|selected| {
                inventory
                    .documents
                    .iter()
                    .position(|document| *document == selected)
            })
            .unwrap_or(0);
        let dirty = matches!(
            projection.persistence(),
            PersistenceProjectionV1::Dirty | PersistenceProjectionV1::Indeterminate
        );
        Some(EditorFrame {
            selection_cells: vec![Vec::new(); lines.len()],
            lines,
            selection_count: 1,
            cursor: Some((
                u16::try_from(column).unwrap_or(u16::MAX),
                u16::try_from(line.saturating_sub(first)).unwrap_or(u16::MAX),
            )),
            status: format!(
                "Service {:?} / {:?}",
                projection.snapshot_freshness(),
                projection.persistence()
            ),
            active,
            documents: inventory.documents.len(),
            dirty,
            editing_locked: projection.snapshot_freshness() != ProjectionFreshnessV1::Current,
        })
    }

    fn ensure_selection(&mut self) -> Result<(), ServiceEditorError> {
        let projection = self.workspace.projection();
        if projection.selected().is_some() && projection.snapshot().is_some() {
            return Ok(());
        }
        let Some(first) = projection
            .inventory()
            .and_then(|inventory| inventory.documents.first().copied())
        else {
            return Ok(());
        };
        if !projection.view_open() {
            self.workspace.open_view()?;
        } else {
            self.workspace.select_document(first)?;
        }
        self.workspace.refresh_snapshot()?;
        self.cursor = 0;
        Ok(())
    }

    fn refresh(&mut self) -> Result<(), ServiceEditorError> {
        self.workspace.refresh_inventory()?;
        self.ensure_selection()?;
        if self.workspace.projection().selected().is_some() {
            self.workspace.refresh_snapshot()?;
        }
        Ok(())
    }

    fn next_document(&mut self) -> Result<(), ServiceEditorError> {
        if self.workspace.projection().inventory_freshness() != ProjectionFreshnessV1::Current {
            self.workspace.refresh_inventory()?;
        }
        let projection = self.workspace.projection();
        let inventory = projection.inventory().ok_or(ServiceEditorError(
            vellum_sdk_client::workspace::WorkspaceClientErrorV1::Projection(
                vellum_sdk_client::workspace::WorkspaceProjectionRefusalV1::InventoryRequired,
            ),
        ))?;
        if inventory.documents.is_empty() {
            return Ok(());
        }
        let current = projection
            .selected()
            .and_then(|selected| {
                inventory
                    .documents
                    .iter()
                    .position(|document| *document == selected)
            })
            .unwrap_or(0);
        let next = inventory.documents[(current + 1) % inventory.documents.len()];
        self.select(next)
    }

    fn select(&mut self, document: WorkspaceDocumentIdV1) -> Result<(), ServiceEditorError> {
        self.workspace.select_document(document)?;
        self.workspace.refresh_snapshot()?;
        self.cursor = 0;
        Ok(())
    }

    fn source(&self) -> &str {
        self.workspace
            .projection()
            .snapshot()
            .map_or("", |snapshot| snapshot.text.as_ref())
    }

    fn cursor_u64(&self) -> Result<u64, ServiceEditorError> {
        to_u64(self.cursor)
    }

    fn previous_boundary(&self) -> Option<usize> {
        self.source()[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(at, _)| at)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.source()[self.cursor..]
            .chars()
            .next()
            .map(|character| self.cursor + character.len_utf8())
    }

    fn current_line(&self) -> (usize, usize) {
        let source = self.source().as_bytes();
        let start = source[..self.cursor]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let end = source[self.cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |at| self.cursor + at);
        (start, end)
    }

    fn move_vertical(&mut self, down: bool) {
        let (start, end) = self.current_line();
        let column = self.source()[start..self.cursor].chars().count();
        let target = if down {
            if end == self.source().len() {
                return;
            }
            let start = end + 1;
            let end = self.source()[start..]
                .find('\n')
                .map_or(self.source().len(), |at| start + at);
            (start, end)
        } else {
            if start == 0 {
                return;
            }
            let end = start - 1;
            let start = self.source()[..end].rfind('\n').map_or(0, |at| at + 1);
            (start, end)
        };
        self.cursor = self.source()[target.0..target.1]
            .char_indices()
            .nth(column)
            .map_or(target.1, |(at, _)| target.0 + at);
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.source().len());
        while !self.source().is_char_boundary(self.cursor) {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
}

fn to_u64(value: usize) -> Result<u64, ServiceEditorError> {
    u64::try_from(value).map_err(|_| {
        ServiceEditorError(WorkspaceClientErrorV1::Access(
            vellum_sdk_client::protocol::ServiceAccessErrorV1::Limit,
        ))
    })
}

fn line_and_column(source: &str, cursor: usize) -> (usize, usize) {
    let prefix = &source[..cursor.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let tail = prefix.rsplit('\n').next().unwrap_or("");
    (line, UnicodeWidthStr::width(tail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Arc};
    use vellum_sdk_client::protocol::{
        ClientId, CommandV1, OutcomeV1, ReplyV1, RequestV1, SaveApplicationV1, SaveReceiptV1,
        SaveSynchronizationV1, ServiceAccessErrorV1, ServiceControlErrorV1, SnapshotV1,
        VersionWitness, WorkspaceIdV1, WorkspaceInventoryV1,
    };

    struct ScriptedPort {
        replies: VecDeque<Result<ReplyV1, ServiceAccessErrorV1>>,
        commands: Vec<CommandV1>,
    }

    impl ServicePort for ScriptedPort {
        fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
            Ok(ClientId::new(1).unwrap())
        }

        fn execute(&mut self, request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
            self.commands.push(request.command);
            self.replies.pop_front().unwrap()
        }

        fn disconnect(&mut self, _: ClientId) -> Result<(), ServiceControlErrorV1> {
            Ok(())
        }
    }

    #[test]
    fn line_projection_uses_terminal_cells_not_source_bytes() {
        assert_eq!(line_and_column("a界\nnext", "a界".len()), (0, 3));
        assert_eq!(line_and_column("a界\nnext", "a界\nn".len()), (1, 1));
    }

    #[test]
    fn service_input_mapping_is_closed_over_the_supported_controller_surface() {
        assert_eq!(
            translate_service_input(crate::Input::Edit(vellum_editor::Action::Insert(
                "x".into()
            ))),
            ServiceInput::Apply(ServiceAction::Insert("x".into()))
        );
        assert_eq!(
            translate_service_input(crate::Input::Edit(vellum_editor::Action::Undo)),
            ServiceInput::Unsupported
        );
        assert_eq!(
            translate_service_input(crate::Input::Open),
            ServiceInput::Open
        );
    }

    #[test]
    fn service_editor_opens_edits_saves_and_returns_its_transport() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let document = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
        let initial = SnapshotV1 {
            witness: VersionWitness::new(3).unwrap(),
            revision: 4,
            text: Arc::from("A\n"),
        };
        let successor = SnapshotV1 {
            witness: VersionWitness::new(5).unwrap(),
            revision: 5,
            text: Arc::from("A界\n"),
        };
        let port = ScriptedPort {
            replies: [
                Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(
                    WorkspaceInventoryV1 {
                        workspace,
                        documents: vec![document],
                        roots: Vec::new(),
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::ViewOpened(
                    vellum_sdk_client::protocol::ViewOpenReceiptV1 {
                        document,
                        status: vellum_sdk_client::protocol::ViewOpenStatusV1::Opened,
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(initial))),
                Ok(ReplyV1::Accepted(OutcomeV1::Replaced(successor))),
                Ok(ReplyV1::Accepted(OutcomeV1::Saved(SaveReceiptV1 {
                    application: SaveApplicationV1::Written,
                    synchronization: SaveSynchronizationV1::Succeeded,
                    no_future_effects: true,
                    quiescent: true,
                    adapter_failure: false,
                    cleanup_failed: false,
                    baseline_published: true,
                    outcome_unknown: false,
                }))),
            ]
            .into_iter()
            .collect(),
            commands: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();
        editor.apply(ServiceAction::Right).unwrap();
        editor.apply(ServiceAction::Insert("界".into())).unwrap();
        assert_eq!(editor.frame(4).unwrap().cursor, Some((3, 0)));
        assert_eq!(
            editor.workspace().projection().persistence(),
            PersistenceProjectionV1::Dirty
        );
        editor.apply(ServiceAction::Save).unwrap();
        assert_eq!(
            editor.workspace().projection().persistence(),
            PersistenceProjectionV1::Clean
        );
        let port = editor.disconnect().unwrap();
        assert!(matches!(port.commands[0], CommandV1::WorkspaceInventory));
        assert!(matches!(port.commands[1], CommandV1::OpenView));
        assert!(matches!(port.commands[2], CommandV1::Snapshot));
        assert!(matches!(port.commands[3], CommandV1::Replace { .. }));
        assert!(matches!(port.commands[4], CommandV1::Save { .. }));
    }
}
