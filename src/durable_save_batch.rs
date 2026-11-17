//! Bounded, independently receipted durable saves for an explicit workset.

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use vellum_sdk_client::protocol::{DurableOperationRefV1, ServicePort};

use crate::{DurableSaveCommandV1, DurableSaveOutcomeV1, run_durable_save_v1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableSaveTargetV1 {
    pub operation: DurableOperationRefV1,
    pub document_ordinal: u64,
    pub expected_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveBatchInput {
    Arm,
    Confirm,
    Recover,
    Back,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveBatchLifecycle {
    Running,
    CloseRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveChildDisposition {
    NotSent,
    Pending,
    Durable,
    NotDurable,
    Refused,
    Retired,
    RequestConflict,
    OutcomeUnknown,
    AbsentOrForeign,
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveBatchDisposition {
    Proposed,
    Armed,
    Pending,
    Durable,
    Partial,
    OutcomeUnknown,
}

struct Child {
    target: DurableSaveTargetV1,
    disposition: DurableSaveChildDisposition,
}

enum Work {
    ExecuteFrom(usize),
    Recover(usize),
}

enum Reply {
    Started(usize),
    Settled(usize, DurableSaveChildDisposition),
    Done,
}

pub struct DurableSaveBatchComponent {
    work: SyncSender<Work>,
    replies: Receiver<Reply>,
    children: Vec<Child>,
    armed: bool,
    pending: bool,
    selected: usize,
    notice: String,
}

impl DurableSaveBatchComponent {
    pub const MAX_CHILDREN: usize = 16;

    pub fn new_with_connector<T, F, E>(
        connector: F,
        targets: Vec<DurableSaveTargetV1>,
    ) -> Result<Self, &'static str>
    where
        T: ServicePort + 'static,
        F: Fn() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
    {
        if targets.is_empty() {
            return Err("Vellum durable-save batch requires at least one exact target");
        }
        if targets.len() > Self::MAX_CHILDREN {
            return Err("Vellum durable-save batch exceeds the 16-document bound");
        }
        if targets.iter().enumerate().any(|(index, target)| {
            targets[..index]
                .iter()
                .any(|prior| prior.document_ordinal == target.document_ordinal)
        }) {
            return Err("Vellum durable-save batch requires unique documents");
        }
        let worker_targets = targets.clone();
        let (work_tx, work_rx) = sync_channel(1);
        let (reply_tx, reply_rx) = sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(work) = work_rx.recv() {
                match work {
                    Work::ExecuteFrom(start) => {
                        for (index, target) in
                            worker_targets.iter().copied().enumerate().skip(start)
                        {
                            if reply_tx.send(Reply::Started(index)).is_err() {
                                return;
                            }
                            let disposition = connector()
                                .map_or(DurableSaveChildDisposition::Failed, |transport| {
                                    run(transport, DurableSaveCommandV1::Execute, target)
                                });
                            let stop = matches!(
                                disposition,
                                DurableSaveChildDisposition::OutcomeUnknown
                                    | DurableSaveChildDisposition::Failed
                            );
                            if reply_tx.send(Reply::Settled(index, disposition)).is_err() {
                                return;
                            }
                            if stop {
                                break;
                            }
                        }
                        if reply_tx.send(Reply::Done).is_err() {
                            return;
                        }
                    }
                    Work::Recover(index) => {
                        if reply_tx.send(Reply::Started(index)).is_err() {
                            return;
                        }
                        let disposition = connector().map_or(
                            DurableSaveChildDisposition::Unavailable,
                            |transport| {
                                run(
                                    transport,
                                    DurableSaveCommandV1::Lookup,
                                    worker_targets[index],
                                )
                            },
                        );
                        if reply_tx.send(Reply::Settled(index, disposition)).is_err() {
                            return;
                        }
                        if reply_tx.send(Reply::Done).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Self {
            work: work_tx,
            replies: reply_rx,
            children: targets
                .into_iter()
                .map(|target| Child {
                    target,
                    disposition: DurableSaveChildDisposition::NotSent,
                })
                .collect(),
            armed: false,
            pending: false,
            selected: 0,
            notice: "Review every exact document/revision; c arms, Enter starts once".into(),
        })
    }

    pub fn poll(&mut self) {
        loop {
            match self.replies.try_recv() {
                Ok(Reply::Started(index)) => {
                    self.selected = index;
                    self.children[index].disposition = DurableSaveChildDisposition::Pending;
                    self.notice = format!(
                        "Vellum durable save {}/{} pending",
                        index + 1,
                        self.children.len()
                    );
                }
                Ok(Reply::Settled(index, disposition)) => {
                    self.children[index].disposition = disposition;
                    self.notice =
                        format!("Vellum document {} settled as {disposition:?}", index + 1);
                }
                Ok(Reply::Done) => {
                    self.pending = false;
                    self.notice = format!(
                        "Vellum run settled: {} durable, {} not sent",
                        self.durable_count(),
                        self.not_sent_count()
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.pending {
                        if let Some(child) = self
                            .children
                            .iter_mut()
                            .find(|child| child.disposition == DurableSaveChildDisposition::Pending)
                        {
                            child.disposition = DurableSaveChildDisposition::OutcomeUnknown;
                        }
                        self.pending = false;
                        self.notice = "Vellum batch worker disconnected with an exact save pending; outcome unknown".into();
                    }
                    break;
                }
            }
        }
    }

    pub fn update(
        &mut self,
        input: DurableSaveBatchInput,
        focused: bool,
    ) -> DurableSaveBatchLifecycle {
        if !focused {
            return DurableSaveBatchLifecycle::Running;
        }
        match input {
            DurableSaveBatchInput::Close if self.pending || self.recovery_required() => {
                self.notice = "Vellum durable save is unresolved; exact receipt lookup is required before exit".into();
            }
            DurableSaveBatchInput::Close => return DurableSaveBatchLifecycle::CloseRequested,
            DurableSaveBatchInput::Recover if !self.pending => {
                let Some(index) = self.children.iter().position(|child| {
                    matches!(
                        child.disposition,
                        DurableSaveChildDisposition::OutcomeUnknown
                            | DurableSaveChildDisposition::AbsentOrForeign
                            | DurableSaveChildDisposition::Unavailable
                    )
                }) else {
                    self.notice = "No unresolved Vellum receipt is available for lookup".into();
                    return DurableSaveBatchLifecycle::Running;
                };
                if self.work.try_send(Work::Recover(index)).is_ok() {
                    self.pending = true;
                    self.notice = "Exact Vellum durable receipt lookup pending".into();
                }
            }
            DurableSaveBatchInput::Arm if !self.pending && self.can_start() => {
                self.armed = true;
                self.notice = "Vellum durable-save workset armed; Enter starts once".into();
            }
            DurableSaveBatchInput::Confirm if self.armed && self.can_start() => {
                self.armed = false;
                let start = self
                    .children
                    .iter()
                    .position(|child| child.disposition == DurableSaveChildDisposition::NotSent)
                    .unwrap_or(0);
                if self.work.try_send(Work::ExecuteFrom(start)).is_ok() {
                    self.pending = true;
                    self.notice = "Vellum independently receipted durable-save run started".into();
                }
            }
            DurableSaveBatchInput::Back if self.can_start() => {
                self.armed = false;
                self.notice = "Vellum durable-save workset disarmed".into();
            }
            DurableSaveBatchInput::Arm
            | DurableSaveBatchInput::Confirm
            | DurableSaveBatchInput::Recover
            | DurableSaveBatchInput::Back => {}
        }
        DurableSaveBatchLifecycle::Running
    }

    pub fn draw_in(&self, frame: &mut Frame<'_>, area: Rect, focused: bool) {
        let [body, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let title = if focused {
            " Vellum durable-save workset • focused "
        } else {
            " Vellum durable-save workset "
        };
        let mut lines = vec![
            Line::from(format!(
                "{} exact documents • {} durable",
                self.children.len(),
                self.durable_count()
            )),
            Line::from(
                "Persistence is sequential and non-atomic; each child retains its own receipt.",
            ),
            Line::from(""),
        ];
        for (index, child) in self.children.iter().enumerate() {
            let marker = if index == self.selected { ">" } else { " " };
            lines.push(Line::from(format!(
                "{marker} {:>2}. document {} @ revision {}  {:?}",
                index + 1,
                child.target.document_ordinal,
                child.target.expected_revision,
                child.disposition
            )));
        }
        lines.extend([
            Line::from(""),
            Line::from(if self.armed {
                "ARMED — Enter starts once; Esc disarms"
            } else {
                "c arm · Enter confirm/resume · r exact receipt lookup · Esc close"
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

    pub fn disposition(&self) -> DurableSaveBatchDisposition {
        if self.pending {
            DurableSaveBatchDisposition::Pending
        } else if self.recovery_required() {
            DurableSaveBatchDisposition::OutcomeUnknown
        } else if self.armed {
            DurableSaveBatchDisposition::Armed
        } else if self
            .children
            .iter()
            .all(|child| child.disposition == DurableSaveChildDisposition::NotSent)
        {
            DurableSaveBatchDisposition::Proposed
        } else if self
            .children
            .iter()
            .all(|child| child.disposition == DurableSaveChildDisposition::Durable)
        {
            DurableSaveBatchDisposition::Durable
        } else {
            DurableSaveBatchDisposition::Partial
        }
    }

    pub const fn request_pending(&self) -> bool {
        self.pending
    }

    pub fn recovery_required(&self) -> bool {
        self.children.iter().any(|child| {
            matches!(
                child.disposition,
                DurableSaveChildDisposition::OutcomeUnknown
                    | DurableSaveChildDisposition::AbsentOrForeign
                    | DurableSaveChildDisposition::Unavailable
            )
        })
    }

    pub fn notice(&self) -> &str {
        &self.notice
    }

    pub fn child_dispositions(&self) -> Vec<DurableSaveChildDisposition> {
        self.children
            .iter()
            .map(|child| child.disposition)
            .collect()
    }

    pub fn durable_document_ordinals(&self) -> Vec<u64> {
        self.children
            .iter()
            .filter(|child| child.disposition == DurableSaveChildDisposition::Durable)
            .map(|child| child.target.document_ordinal)
            .collect()
    }

    fn durable_count(&self) -> usize {
        self.children
            .iter()
            .filter(|child| child.disposition == DurableSaveChildDisposition::Durable)
            .count()
    }

    fn not_sent_count(&self) -> usize {
        self.children
            .iter()
            .filter(|child| child.disposition == DurableSaveChildDisposition::NotSent)
            .count()
    }

    fn can_start(&self) -> bool {
        !self.recovery_required()
            && self
                .children
                .iter()
                .any(|child| child.disposition == DurableSaveChildDisposition::NotSent)
    }
}

fn run<T: ServicePort>(
    transport: T,
    command: DurableSaveCommandV1,
    target: DurableSaveTargetV1,
) -> DurableSaveChildDisposition {
    match run_durable_save_v1(
        transport,
        command,
        target.operation,
        target.document_ordinal,
        target.expected_revision,
    ) {
        Ok(report) => match report.outcome {
            DurableSaveOutcomeV1::Completed { .. } if report.outcome.status() == "durable" => {
                DurableSaveChildDisposition::Durable
            }
            DurableSaveOutcomeV1::Completed { .. }
                if report.outcome.status() == "completed_outcome_unknown" =>
            {
                DurableSaveChildDisposition::OutcomeUnknown
            }
            DurableSaveOutcomeV1::Completed { .. } => DurableSaveChildDisposition::NotDurable,
            DurableSaveOutcomeV1::Refused => DurableSaveChildDisposition::Refused,
            DurableSaveOutcomeV1::Retired => DurableSaveChildDisposition::Retired,
            DurableSaveOutcomeV1::RequestConflict => DurableSaveChildDisposition::RequestConflict,
            DurableSaveOutcomeV1::OutcomeUnknown => DurableSaveChildDisposition::OutcomeUnknown,
            DurableSaveOutcomeV1::AbsentOrForeign => DurableSaveChildDisposition::AbsentOrForeign,
            DurableSaveOutcomeV1::Unavailable => DurableSaveChildDisposition::Unavailable,
        },
        Err(_) => DurableSaveChildDisposition::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    use vellum_sdk_client::protocol::{
        ClientId, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
        DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1, OutcomeV1,
        ReplyV1, RequestV1, SaveApplicationV1, SaveReceiptV1, SaveSynchronizationV1,
        ServiceAccessErrorV1, ServiceControlErrorV1, WorkspaceDocumentIdV1, WorkspaceIdV1,
        WorkspaceInventoryV1,
    };

    #[derive(Default)]
    struct State {
        executes: [usize; 3],
        lookups: [usize; 3],
    }

    struct Port {
        state: Arc<Mutex<State>>,
    }

    impl Port {
        fn workspace() -> WorkspaceIdV1 {
            WorkspaceIdV1::new(7).unwrap()
        }

        fn document(ordinal: u64) -> WorkspaceDocumentIdV1 {
            WorkspaceDocumentIdV1::new(Self::workspace(), ordinal).unwrap()
        }

        fn complete(document: WorkspaceDocumentIdV1, revision: u64) -> DurableExecuteOutcomeV1 {
            DurableExecuteOutcomeV1::Completed(DurableEffectReceiptV1::DocumentSaved(
                vellum_sdk_client::protocol::DurableSaveReceiptV1 {
                    document,
                    source_revision: revision,
                    persistence: SaveReceiptV1 {
                        application: SaveApplicationV1::Written,
                        synchronization: SaveSynchronizationV1::Succeeded,
                        no_future_effects: true,
                        quiescent: true,
                        adapter_failure: false,
                        cleanup_failed: false,
                        baseline_published: true,
                        outcome_unknown: false,
                    },
                },
            ))
        }

        fn intent(intent: DurableEffectIntentV1) -> (WorkspaceDocumentIdV1, u64, usize) {
            let DurableEffectIntentV1::SaveDocument {
                document,
                expected_revision,
            } = intent
            else {
                panic!("unexpected durable intent")
            };
            let index = usize::try_from(document.get() - 11).unwrap();
            (document, expected_revision, index)
        }
    }

    impl ServicePort for Port {
        fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
            Ok(ClientId::new(3).unwrap())
        }

        fn execute(&mut self, _request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
            Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(
                WorkspaceInventoryV1 {
                    workspace: Self::workspace(),
                    documents: (11..=13).map(Self::document).collect(),
                    roots: Vec::new(),
                },
            )))
        }

        fn disconnect(&mut self, _client: ClientId) -> Result<(), ServiceControlErrorV1> {
            Ok(())
        }

        fn execute_durable(
            &mut self,
            request: DurableExecuteRequestV1,
        ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
            let (document, revision, index) = Self::intent(request.intent);
            let mut state = self.state.lock().unwrap();
            state.executes[index] += 1;
            if index == 1 {
                Err(ServiceAccessErrorV1::OutcomeUnknown)
            } else {
                Ok(Self::complete(document, revision))
            }
        }

        fn lookup_durable_receipt(
            &mut self,
            lookup: DurableReceiptLookupV1,
        ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
            let (document, revision, index) = Self::intent(lookup.intent);
            self.state.lock().unwrap().lookups[index] += 1;
            Ok(DurableReceiptLookupOutcomeV1::Complete(Self::complete(
                document, revision,
            )))
        }
    }

    fn settle(component: &mut DurableSaveBatchComponent) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while component.request_pending() {
            component.poll();
            assert!(
                Instant::now() < deadline,
                "durable-save batch did not settle"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn exact_recovery_precedes_explicit_resume_of_not_sent_children() {
        let state = Arc::new(Mutex::new(State::default()));
        let connector_state = Arc::clone(&state);
        let targets = (0_u8..3)
            .map(|index| DurableSaveTargetV1 {
                operation: DurableOperationRefV1::new([index + 1; 16]).unwrap(),
                document_ordinal: u64::from(index) + 11,
                expected_revision: u64::from(index) + 21,
            })
            .collect();
        let mut component = DurableSaveBatchComponent::new_with_connector(
            move || {
                Ok::<_, ()>(Port {
                    state: Arc::clone(&connector_state),
                })
            },
            targets,
        )
        .unwrap();
        component.update(DurableSaveBatchInput::Arm, true);
        component.update(DurableSaveBatchInput::Confirm, true);
        settle(&mut component);
        assert_eq!(
            component.child_dispositions(),
            vec![
                DurableSaveChildDisposition::Durable,
                DurableSaveChildDisposition::OutcomeUnknown,
                DurableSaveChildDisposition::NotSent,
            ]
        );
        assert_eq!(state.lock().unwrap().executes, [1, 1, 0]);

        component.update(DurableSaveBatchInput::Recover, true);
        settle(&mut component);
        assert_eq!(
            component.child_dispositions(),
            vec![
                DurableSaveChildDisposition::Durable,
                DurableSaveChildDisposition::Durable,
                DurableSaveChildDisposition::NotSent,
            ]
        );
        assert_eq!(state.lock().unwrap().lookups, [0, 1, 0]);

        component.update(DurableSaveBatchInput::Confirm, true);
        assert!(
            !component.request_pending(),
            "resume must require re-arming"
        );
        component.update(DurableSaveBatchInput::Arm, true);
        component.update(DurableSaveBatchInput::Confirm, true);
        settle(&mut component);
        assert_eq!(
            component.child_dispositions(),
            vec![
                DurableSaveChildDisposition::Durable,
                DurableSaveChildDisposition::Durable,
                DurableSaveChildDisposition::Durable,
            ]
        );
        assert_eq!(state.lock().unwrap().executes, [1, 1, 1]);
    }
}
