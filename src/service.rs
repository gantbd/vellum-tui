use unicode_width::UnicodeWidthStr;
use vellum_editor::EditorFrame;
use vellum_sdk_client::{
    protocol::{
        MAX_WORKSPACE_EDIT_PAYLOAD_BYTES_V1, MAX_WORKSPACE_EDIT_REPLACEMENTS_V1, RefusalV1,
        ServicePort, WorkspaceDocumentEditV1, WorkspaceDocumentIdV1, WorkspaceReplacementV1,
    },
    workspace::{
        PersistenceProjectionV1, ProjectionFreshnessV1, WorkspaceClientErrorV1, WorkspaceClientV1,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtomicReplaceDocumentPreviewV1 {
    pub document_ordinal: u64,
    pub expected_revision: u64,
    pub matches: usize,
}

/// Exact, bounded preview for one owner-atomic in-memory publication. Search
/// and replacement bytes are intentionally absent from Debug and host-facing
/// metadata; the private edits are submitted only to Vellum's public client.
#[derive(Clone, Eq, PartialEq)]
pub struct AtomicReplacePreviewV1 {
    pub documents: Vec<AtomicReplaceDocumentPreviewV1>,
    pub total_matches: usize,
    pub(crate) edits: Vec<WorkspaceDocumentEditV1>,
}

impl std::fmt::Debug for AtomicReplacePreviewV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AtomicReplacePreviewV1")
            .field("documents", &self.documents)
            .field("total_matches", &self.total_matches)
            .field("edits", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtomicReplaceCommitV1 {
    pub generation: u64,
    pub changed_documents: usize,
}

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
    ToggleDocumentSelection,
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
    Find,
    Apply(ServiceAction),
    Unsupported,
}

pub fn translate_service_input(input: crate::Input) -> ServiceInput {
    match input {
        crate::Input::Quit => ServiceInput::Quit,
        crate::Input::CancelQuit => ServiceInput::CancelQuit,
        crate::Input::Open => ServiceInput::Open,
        crate::Input::Find => ServiceInput::Find,
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
    documents: Vec<DocumentUiState>,
    identity_renewed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DocumentUiState {
    document: WorkspaceDocumentIdV1,
    cursor: usize,
    persistence: PersistenceProjectionV1,
    selected: bool,
}

impl<T: ServicePort> ServiceEditor<T> {
    pub fn open(mut workspace: WorkspaceClientV1<T>) -> Result<Self, ServiceEditorError> {
        workspace.refresh_inventory()?;
        let mut editor = Self {
            workspace,
            cursor: 0,
            documents: Vec::new(),
            identity_renewed: false,
        };
        editor.ensure_selection()?;
        editor.reconcile_documents();
        editor.sync_current_state();
        Ok(editor)
    }

    pub const fn workspace(&self) -> &WorkspaceClientV1<T> {
        &self.workspace
    }

    pub const fn issued_requests(&self) -> u64 {
        self.workspace.issued_requests()
    }

    /// Renews the bounded owner-client identity without discarding local
    /// cursor/workset state or the owner-backed workspace projection.
    pub fn renew_identity(&mut self) -> Result<(), WorkspaceClientErrorV1> {
        self.workspace.renew_identity()?;
        self.reconcile_documents();
        self.clamp_cursor();
        self.sync_current_state();
        Ok(())
    }

    /// Returns whether the preceding editor action renewed its bounded service
    /// identity. Presentation hosts can turn this into content-free telemetry
    /// without learning document bytes or replay identities.
    pub fn take_identity_renewal(&mut self) -> bool {
        std::mem::take(&mut self.identity_renewed)
    }

    pub fn disconnect(
        self,
    ) -> Result<T, vellum_sdk_client::workspace::WorkspaceDisconnectFailureV1<T>> {
        self.workspace.disconnect()
    }

    pub fn apply(&mut self, action: ServiceAction) -> Result<(), ServiceEditorError> {
        self.renew_before_capacity()?;
        let result = (|| {
            match action {
                ServiceAction::Insert(inserted) => {
                    let at = self.cursor_u64()?;
                    let bytes = inserted.len();
                    self.workspace.replace(at, at, inserted)?;
                    self.cursor = self.cursor.saturating_add(bytes);
                }
                ServiceAction::Backspace => {
                    let Some(start) = self.previous_boundary() else {
                        return Ok::<(), ServiceEditorError>(());
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
                ServiceAction::ToggleDocumentSelection => self.toggle_document_selection(),
                ServiceAction::Save => self.workspace.save()?,
                ServiceAction::Refresh => self.refresh()?,
            }
            Ok(())
        })();
        self.clamp_cursor();
        self.sync_current_state();
        result
    }

    fn renew_before_capacity(&mut self) -> Result<(), ServiceEditorError> {
        const RENEW_AT_REQUESTS: u64 = 192;
        if self.workspace.issued_requests() < RENEW_AT_REQUESTS {
            return Ok(());
        }
        self.renew_identity()?;
        self.identity_renewed = true;
        Ok(())
    }

    pub fn admit_file(&mut self, locator: String) -> Result<(), ServiceEditorError> {
        self.workspace.admit_file(locator)?;
        self.workspace.refresh_inventory()?;
        self.reconcile_documents();
        self.ensure_selection()
    }

    pub fn admit_directory(&mut self, locator: String) -> Result<(), ServiceEditorError> {
        self.workspace.admit_directory(locator)?;
        self.workspace.refresh_inventory()?;
        self.reconcile_documents();
        self.ensure_selection()
    }

    /// Moves the presentation cursor to the end of the next literal match in
    /// the current owner snapshot. This is a local projection operation: it
    /// neither edits source nor claims a durable owner-side search result.
    pub fn find_literal(&mut self, query: &str) -> Result<bool, ServiceEditorError> {
        if query.is_empty() {
            return Err(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                RefusalV1::InputLimit,
            )));
        }
        if self.workspace.projection().snapshot_freshness() != ProjectionFreshnessV1::Current {
            self.workspace.refresh_snapshot()?;
        }
        let text = self
            .workspace
            .projection()
            .snapshot()
            .ok_or(ServiceEditorError(WorkspaceClientErrorV1::Projection(
                vellum_sdk_client::workspace::WorkspaceProjectionRefusalV1::SelectionRequired,
            )))?
            .text
            .clone();
        let start = self.cursor.min(text.len());
        let found = text[start..]
            .find(query)
            .map(|offset| start + offset)
            .or_else(|| text[..start].find(query));
        if let Some(found) = found {
            self.cursor = found.saturating_add(query.len()).min(text.len());
            self.sync_current_state();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn poll(&mut self) -> Result<(), ServiceEditorError> {
        self.workspace.poll_changes()?;
        if self.workspace.projection().inventory_freshness() == ProjectionFreshnessV1::Stale {
            self.workspace.refresh_inventory()?;
            self.reconcile_documents();
        }
        if self.workspace.projection().snapshot_freshness() == ProjectionFreshnessV1::Stale
            && self.workspace.projection().persistence() != PersistenceProjectionV1::Indeterminate
        {
            self.workspace.refresh_snapshot()?;
        }
        self.clamp_cursor();
        self.sync_current_state();
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
        let persistence = self.current_persistence();
        let dirty = matches!(
            persistence,
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
                "Service {:?} / {:?} • {} selected",
                projection.snapshot_freshness(),
                persistence,
                self.selected_document_count(),
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
        self.reconcile_documents();
        self.ensure_selection()?;
        if self.workspace.projection().selected().is_some() {
            self.workspace.refresh_snapshot()?;
        }
        Ok(())
    }

    fn next_document(&mut self) -> Result<(), ServiceEditorError> {
        if self.workspace.projection().inventory_freshness() != ProjectionFreshnessV1::Current {
            self.workspace.refresh_inventory()?;
            self.reconcile_documents();
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
        self.store_cursor();
        self.workspace.select_document(document)?;
        self.workspace.refresh_snapshot()?;
        self.cursor = self
            .documents
            .iter()
            .find(|state| state.document == document)
            .map_or(0, |state| state.cursor);
        Ok(())
    }

    /// Aggregate exit truth across the complete bounded workspace, not merely
    /// the currently visible document.
    pub fn aggregate_persistence(&self) -> PersistenceProjectionV1 {
        if self
            .documents
            .iter()
            .any(|state| state.persistence == PersistenceProjectionV1::Indeterminate)
        {
            PersistenceProjectionV1::Indeterminate
        } else if self
            .documents
            .iter()
            .any(|state| state.persistence == PersistenceProjectionV1::Dirty)
        {
            PersistenceProjectionV1::Dirty
        } else if self.documents.is_empty() {
            PersistenceProjectionV1::Unknown
        } else {
            PersistenceProjectionV1::Clean
        }
    }

    pub fn selected_document_count(&self) -> usize {
        let count = self.documents.iter().filter(|state| state.selected).count();
        if count == 0 && !self.documents.is_empty() {
            1
        } else {
            count
        }
    }

    pub fn selected_documents(&self) -> Vec<WorkspaceDocumentIdV1> {
        let selected = self
            .documents
            .iter()
            .filter(|state| state.selected)
            .map(|state| state.document)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            self.workspace.projection().selected().into_iter().collect()
        } else {
            selected
        }
    }

    pub fn current_document(&self) -> Option<WorkspaceDocumentIdV1> {
        self.workspace.projection().selected()
    }

    /// Re-observes every selected document and returns exact owner ordinals and
    /// source revisions for a separately guarded durable-save workset.
    pub fn prepare_selected_save_targets(&mut self) -> Result<Vec<(u64, u64)>, ServiceEditorError> {
        let selected = self.selected_documents();
        if selected.iter().any(|document| {
            self.documents.iter().any(|state| {
                state.document == *document
                    && state.persistence == PersistenceProjectionV1::Indeterminate
            })
        }) {
            return Err(ServiceEditorError(WorkspaceClientErrorV1::Access(
                vellum_sdk_client::protocol::ServiceAccessErrorV1::OutcomeUnknown,
            )));
        }
        let original = self.current_document();
        let result = (|| {
            let mut targets = Vec::with_capacity(selected.len());
            for document in selected {
                if self.current_document() != Some(document) {
                    self.select(document)?;
                } else if self.workspace.projection().snapshot_freshness()
                    != ProjectionFreshnessV1::Current
                {
                    self.workspace.refresh_snapshot()?;
                }
                let revision = self
                    .workspace
                    .projection()
                    .snapshot()
                    .ok_or(ServiceEditorError(WorkspaceClientErrorV1::Projection(
                        vellum_sdk_client::workspace::WorkspaceProjectionRefusalV1::SelectionRequired,
                    )))?
                    .revision;
                targets.push((document.get(), revision));
            }
            Ok(targets)
        })();
        if let Some(original) = original
            && self.current_document() != Some(original)
        {
            let restored = self.select(original);
            if result.is_ok() {
                restored?;
            }
        }
        result
    }

    /// Re-observes the complete marked set and prepares one literal replace-all
    /// transaction over every match. The preview is exact but has no authority;
    /// commit still binds all owner revisions atomically.
    pub fn prepare_atomic_replace(
        &mut self,
        query: &str,
        replacement: &str,
    ) -> Result<AtomicReplacePreviewV1, ServiceEditorError> {
        if query.is_empty() || query == replacement {
            return Err(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                RefusalV1::InputLimit,
            )));
        }
        let selected = self.selected_documents();
        let original = self.current_document();
        let result = (|| {
            let mut documents = Vec::with_capacity(selected.len());
            let mut edits = Vec::with_capacity(selected.len());
            let mut total_matches = 0usize;
            let mut payload_bytes = 4usize;
            for document in selected {
                if self.current_document() != Some(document) {
                    self.select(document)?;
                } else if self.workspace.projection().snapshot_freshness()
                    != ProjectionFreshnessV1::Current
                {
                    self.workspace.refresh_snapshot()?;
                }
                let snapshot = self
                    .workspace
                    .projection()
                    .snapshot()
                    .ok_or(ServiceEditorError(WorkspaceClientErrorV1::Projection(
                    vellum_sdk_client::workspace::WorkspaceProjectionRefusalV1::SelectionRequired,
                )))?;
                let matches = snapshot.text.match_indices(query).collect::<Vec<_>>();
                if matches.len() > MAX_WORKSPACE_EDIT_REPLACEMENTS_V1 {
                    return Err(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                        RefusalV1::InputLimit,
                    )));
                }
                documents.push(AtomicReplaceDocumentPreviewV1 {
                    document_ordinal: document.get(),
                    expected_revision: snapshot.revision,
                    matches: matches.len(),
                });
                if matches.is_empty() {
                    continue;
                }
                let mut replacements = Vec::with_capacity(matches.len());
                for (start, matched) in matches {
                    payload_bytes = payload_bytes
                        .checked_add(20)
                        .and_then(|bytes| bytes.checked_add(replacement.len()))
                        .filter(|bytes| *bytes <= MAX_WORKSPACE_EDIT_PAYLOAD_BYTES_V1)
                        .ok_or(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                            RefusalV1::InputLimit,
                        )))?;
                    replacements.push(WorkspaceReplacementV1 {
                        start: to_u64(start)?,
                        end: to_u64(start + matched.len())?,
                        inserted: replacement.to_owned(),
                    });
                }
                total_matches =
                    total_matches
                        .checked_add(replacements.len())
                        .ok_or(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                            RefusalV1::InputLimit,
                        )))?;
                edits.push(WorkspaceDocumentEditV1 {
                    document,
                    expected_revision: snapshot.revision,
                    replacements,
                });
            }
            if edits.is_empty() {
                return Err(ServiceEditorError(WorkspaceClientErrorV1::Refused(
                    RefusalV1::InputLimit,
                )));
            }
            Ok(AtomicReplacePreviewV1 {
                documents,
                total_matches,
                edits,
            })
        })();
        if let Some(original) = original
            && self.current_document() != Some(original)
        {
            let restored = self.select(original);
            if result.is_ok() {
                restored?;
            }
        }
        result
    }

    pub fn commit_atomic_replace(
        &mut self,
        preview: &AtomicReplacePreviewV1,
    ) -> Result<AtomicReplaceCommitV1, ServiceEditorError> {
        let participants = preview
            .edits
            .iter()
            .map(|edit| edit.document)
            .collect::<Vec<_>>();
        match self.workspace.workspace_edit(preview.edits.clone()) {
            Ok(receipt) => {
                for edited in &receipt.documents {
                    if let Some(state) = self
                        .documents
                        .iter_mut()
                        .find(|state| state.document == edited.document)
                    {
                        state.persistence = PersistenceProjectionV1::Dirty;
                        state.cursor = state.cursor.min(edited.snapshot.text.len());
                    }
                }
                self.clamp_cursor();
                self.sync_current_state();
                Ok(AtomicReplaceCommitV1 {
                    generation: receipt.generation,
                    changed_documents: receipt.documents.len(),
                })
            }
            Err(error) => {
                let persistence = match error {
                    WorkspaceClientErrorV1::Access(
                        vellum_sdk_client::protocol::ServiceAccessErrorV1::OutcomeUnknown,
                    ) => Some(PersistenceProjectionV1::Indeterminate),
                    WorkspaceClientErrorV1::Refused(RefusalV1::Stale) => {
                        Some(PersistenceProjectionV1::Unknown)
                    }
                    _ => None,
                };
                if let Some(persistence) = persistence {
                    for state in &mut self.documents {
                        if participants.contains(&state.document) {
                            state.persistence = persistence;
                        }
                    }
                }
                self.sync_current_state();
                Err(error.into())
            }
        }
    }

    /// Admits only exact durable child receipts already verified by the batch
    /// component. Other documents keep their prior local persistence truth.
    pub fn admit_durable_saves(&mut self, ordinals: &[u64]) {
        for state in &mut self.documents {
            if ordinals.contains(&state.document.get()) {
                state.persistence = PersistenceProjectionV1::Clean;
            }
        }
        if let Some(document) = self.current_document()
            && ordinals.contains(&document.get())
        {
            self.workspace.admit_durable_document(document);
        }
    }

    fn toggle_document_selection(&mut self) {
        let Some(document) = self.current_document() else {
            return;
        };
        if let Some(state) = self
            .documents
            .iter_mut()
            .find(|state| state.document == document)
        {
            state.selected = !state.selected;
        }
    }

    fn reconcile_documents(&mut self) {
        let Some(inventory) = self.workspace.projection().inventory() else {
            return;
        };
        self.documents
            .retain(|state| inventory.documents.contains(&state.document));
        for document in &inventory.documents {
            if !self
                .documents
                .iter()
                .any(|state| state.document == *document)
            {
                self.documents.push(DocumentUiState {
                    document: *document,
                    cursor: 0,
                    persistence: PersistenceProjectionV1::Clean,
                    selected: false,
                });
            }
        }
    }

    fn store_cursor(&mut self) {
        let Some(document) = self.current_document() else {
            return;
        };
        if let Some(state) = self
            .documents
            .iter_mut()
            .find(|state| state.document == document)
        {
            state.cursor = self.cursor;
        }
    }

    fn sync_current_state(&mut self) {
        let Some(document) = self.current_document() else {
            return;
        };
        let projected = self.workspace.projection().persistence();
        if let Some(state) = self
            .documents
            .iter_mut()
            .find(|state| state.document == document)
        {
            state.cursor = self.cursor;
            if projected != PersistenceProjectionV1::Unknown {
                state.persistence = projected;
            }
        }
    }

    fn current_persistence(&self) -> PersistenceProjectionV1 {
        self.current_document()
            .and_then(|document| {
                self.documents
                    .iter()
                    .find(|state| state.document == document)
            })
            .map_or(PersistenceProjectionV1::Unknown, |state| state.persistence)
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
        request_ids: Vec<u64>,
    }

    impl ServicePort for ScriptedPort {
        fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
            Ok(ClientId::new(1).unwrap())
        }

        fn execute(&mut self, request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
            self.request_ids.push(request.request.get());
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
        assert_eq!(
            translate_service_input(crate::Input::Find),
            ServiceInput::Find
        );
    }

    #[test]
    fn service_find_moves_the_local_cursor_without_editing_or_owner_effects() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let document = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
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
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(SnapshotV1 {
                    witness: VersionWitness::new(3).unwrap(),
                    revision: 4,
                    text: Arc::from("alpha beta alpha"),
                }))),
            ]
            .into_iter()
            .collect(),
            commands: Vec::new(),
            request_ids: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();

        assert_eq!(editor.issued_requests(), 3);
        assert!(editor.find_literal("alpha").unwrap());
        assert_eq!(editor.frame(4).unwrap().cursor, Some((5, 0)));
        assert!(editor.find_literal("alpha").unwrap());
        assert_eq!(editor.frame(4).unwrap().cursor, Some((16, 0)));
        assert!(!editor.find_literal("missing").unwrap());
        assert_eq!(editor.issued_requests(), 3);
        assert_eq!(
            editor.aggregate_persistence(),
            PersistenceProjectionV1::Clean
        );
    }

    #[test]
    fn identity_renewal_preserves_editor_projection_cursor_and_resets_requests() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let document = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
        let snapshot = SnapshotV1 {
            witness: VersionWitness::new(3).unwrap(),
            revision: 4,
            text: Arc::from("alpha"),
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
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot.clone()))),
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
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot))),
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
            request_ids: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();
        editor.apply(ServiceAction::Right).unwrap();
        assert_eq!(editor.issued_requests(), 3);

        editor.renew_identity().unwrap();
        assert_eq!(editor.issued_requests(), 3);
        assert_eq!(editor.frame(4).unwrap().cursor, Some((1, 0)));
        editor.apply(ServiceAction::Save).unwrap();

        let port = editor.disconnect().unwrap();
        assert_eq!(port.request_ids, vec![1, 2, 3, 1, 2, 3, 4]);
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
            request_ids: Vec::new(),
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

    #[test]
    fn service_editor_retains_n_document_selection_cursor_and_dirty_truth() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let first = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
        let second = WorkspaceDocumentIdV1::new(workspace, 10).unwrap();
        let snapshot = |witness, revision, text: &'static str| SnapshotV1 {
            witness: VersionWitness::new(witness).unwrap(),
            revision,
            text: Arc::from(text),
        };
        let port = ScriptedPort {
            replies: [
                Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(
                    WorkspaceInventoryV1 {
                        workspace,
                        documents: vec![first, second],
                        roots: Vec::new(),
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::ViewOpened(
                    vellum_sdk_client::protocol::ViewOpenReceiptV1 {
                        document: first,
                        status: vellum_sdk_client::protocol::ViewOpenStatusV1::Opened,
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    1, 1, "one",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::Replaced(snapshot(
                    2, 2, "xone",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(second))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    3, 1, "two",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::Replaced(snapshot(
                    4, 2, "ytwo",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(first))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    5, 2, "xone",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(second))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    6, 2, "ytwo",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(first))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    7, 2, "xone",
                )))),
            ]
            .into_iter()
            .collect(),
            commands: Vec::new(),
            request_ids: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();
        editor
            .apply(ServiceAction::ToggleDocumentSelection)
            .unwrap();
        editor.apply(ServiceAction::Insert("x".into())).unwrap();
        editor.apply(ServiceAction::NextDocument).unwrap();
        editor
            .apply(ServiceAction::ToggleDocumentSelection)
            .unwrap();
        editor.apply(ServiceAction::Insert("y".into())).unwrap();

        assert_eq!(editor.selected_document_count(), 2);
        assert_eq!(editor.selected_documents(), vec![first, second]);
        assert_eq!(
            editor.aggregate_persistence(),
            PersistenceProjectionV1::Dirty
        );
        assert!(editor.frame(4).unwrap().status.contains("2 selected"));

        editor.apply(ServiceAction::NextDocument).unwrap();
        assert_eq!(editor.current_document(), Some(first));
        assert_eq!(editor.frame(4).unwrap().cursor, Some((1, 0)));
        assert!(editor.frame(4).unwrap().dirty);

        assert_eq!(
            editor.prepare_selected_save_targets().unwrap(),
            vec![(9, 2), (10, 2)]
        );
        editor.admit_durable_saves(&[9, 10]);
        assert_eq!(
            editor.aggregate_persistence(),
            PersistenceProjectionV1::Clean
        );
        assert_eq!(
            editor.workspace().projection().persistence(),
            PersistenceProjectionV1::Clean
        );
        assert!(!editor.frame(4).unwrap().dirty);
    }

    #[test]
    fn atomic_replace_previews_and_commits_all_marked_documents_once() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let first = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
        let second = WorkspaceDocumentIdV1::new(workspace, 10).unwrap();
        let snapshot = |witness, revision, text: &'static str| SnapshotV1 {
            witness: VersionWitness::new(witness).unwrap(),
            revision,
            text: Arc::from(text),
        };
        let port = ScriptedPort {
            replies: [
                Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(
                    WorkspaceInventoryV1 {
                        workspace,
                        documents: vec![first, second],
                        roots: Vec::new(),
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::ViewOpened(
                    vellum_sdk_client::protocol::ViewOpenReceiptV1 {
                        document: first,
                        status: vellum_sdk_client::protocol::ViewOpenStatusV1::Opened,
                    },
                ))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    1,
                    4,
                    "alpha alpha",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(second))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    2,
                    6,
                    "alpha beta",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(first))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    3,
                    4,
                    "alpha alpha",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::DocumentSelected(second))),
                Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                    4,
                    6,
                    "alpha beta",
                )))),
                Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceEdited(
                    vellum_sdk_client::protocol::WorkspaceEditReceiptV1 {
                        workspace,
                        generation: 11,
                        documents: vec![
                            vellum_sdk_client::protocol::WorkspaceEditedDocumentV1 {
                                document: first,
                                snapshot: snapshot(5, 5, "omega omega"),
                            },
                            vellum_sdk_client::protocol::WorkspaceEditedDocumentV1 {
                                document: second,
                                snapshot: snapshot(6, 7, "omega beta"),
                            },
                        ],
                    },
                ))),
            ]
            .into_iter()
            .collect(),
            commands: Vec::new(),
            request_ids: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();
        editor
            .apply(ServiceAction::ToggleDocumentSelection)
            .unwrap();
        editor.apply(ServiceAction::NextDocument).unwrap();
        editor
            .apply(ServiceAction::ToggleDocumentSelection)
            .unwrap();

        let preview = editor.prepare_atomic_replace("alpha", "omega").unwrap();
        assert_eq!(preview.total_matches, 3);
        assert_eq!(preview.documents[0].matches, 2);
        assert_eq!(preview.documents[1].matches, 1);
        assert_eq!(
            editor.commit_atomic_replace(&preview).unwrap(),
            AtomicReplaceCommitV1 {
                generation: 11,
                changed_documents: 2,
            }
        );
        assert_eq!(
            editor.aggregate_persistence(),
            PersistenceProjectionV1::Dirty
        );
        assert_eq!(editor.frame(4).unwrap().lines, ["omega beta"]);

        let port = editor.disconnect().unwrap();
        let CommandV1::WorkspaceEdit { documents } = &port.commands[9] else {
            panic!("one atomic workspace edit must be submitted")
        };
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].replacements.len(), 2);
        assert_eq!(documents[1].replacements.len(), 1);
        assert_eq!(
            port.commands
                .iter()
                .filter(|command| matches!(command, CommandV1::WorkspaceEdit { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn uncertain_atomic_replace_replays_only_the_exact_retained_transaction() {
        let workspace = WorkspaceIdV1::new(7).unwrap();
        let document = WorkspaceDocumentIdV1::new(workspace, 9).unwrap();
        let initial = SnapshotV1 {
            witness: VersionWitness::new(1).unwrap(),
            revision: 4,
            text: Arc::from("alpha"),
        };
        let successor = SnapshotV1 {
            witness: VersionWitness::new(2).unwrap(),
            revision: 5,
            text: Arc::from("omega"),
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
                Err(ServiceAccessErrorV1::OutcomeUnknown),
                Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceEdited(
                    vellum_sdk_client::protocol::WorkspaceEditReceiptV1 {
                        workspace,
                        generation: 8,
                        documents: vec![vellum_sdk_client::protocol::WorkspaceEditedDocumentV1 {
                            document,
                            snapshot: successor,
                        }],
                    },
                ))),
            ]
            .into_iter()
            .collect(),
            commands: Vec::new(),
            request_ids: Vec::new(),
        };
        let workspace = WorkspaceClientV1::connect(port).unwrap();
        let mut editor = ServiceEditor::open(workspace).unwrap();
        let preview = editor.prepare_atomic_replace("alpha", "omega").unwrap();
        assert_eq!(
            editor.commit_atomic_replace(&preview),
            Err(ServiceEditorError(WorkspaceClientErrorV1::Access(
                ServiceAccessErrorV1::OutcomeUnknown
            )))
        );
        assert_eq!(
            editor.aggregate_persistence(),
            PersistenceProjectionV1::Indeterminate
        );
        assert_eq!(
            editor.commit_atomic_replace(&preview).unwrap(),
            AtomicReplaceCommitV1 {
                generation: 8,
                changed_documents: 1,
            }
        );
        let port = editor.disconnect().unwrap();
        assert_eq!(port.request_ids[3], port.request_ids[4]);
        assert_eq!(port.commands[3], port.commands[4]);
    }
}
