use vellum_sdk_client::{
    client::Client,
    protocol::{
        DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
        DurableOperationRefV1, DurableReceiptLookupOutcomeV1, ReplyV1, SaveApplicationV1,
        SaveSynchronizationV1, ServiceAccessErrorV1, ServicePort, WorkspaceDocumentIdV1,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveCommandV1 {
    Execute,
    Lookup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveDisconnectV1 {
    Clean,
    AbandonedAfterOutcomeUnknown,
}

impl DurableSaveDisconnectV1 {
    pub const fn status(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::AbandonedAfterOutcomeUnknown => "abandoned_after_outcome_unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveOutcomeV1 {
    Completed {
        application: SaveApplicationV1,
        synchronization: SaveSynchronizationV1,
        no_future_effects: bool,
        quiescent: bool,
        adapter_failure: bool,
        cleanup_failed: bool,
        baseline_published: bool,
        outcome_unknown: bool,
    },
    Refused,
    Retired,
    RequestConflict,
    OutcomeUnknown,
    AbsentOrForeign,
    Unavailable,
}

impl DurableSaveOutcomeV1 {
    pub const fn status(self) -> &'static str {
        match self {
            Self::Completed {
                application: SaveApplicationV1::Written,
                synchronization: SaveSynchronizationV1::Succeeded,
                no_future_effects: true,
                quiescent: true,
                adapter_failure: false,
                cleanup_failed: false,
                baseline_published: true,
                outcome_unknown: false,
            } => "durable",
            Self::Completed {
                application: SaveApplicationV1::Unknown,
                ..
            }
            | Self::Completed {
                outcome_unknown: true,
                ..
            } => "completed_outcome_unknown",
            Self::Completed { .. } => "not_durable",
            Self::Refused => "refused",
            Self::Retired => "retired",
            Self::RequestConflict => "request_conflict",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::AbsentOrForeign => "absent_or_foreign",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableSaveReportV1 {
    pub outcome: DurableSaveOutcomeV1,
    pub disconnect: DurableSaveDisconnectV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableSaveCommandErrorV1 {
    Connect,
    InventoryAccess,
    InventoryRefused,
    DocumentAbsent,
    Access,
    ReceiptMismatch,
    Disconnect,
}

impl std::fmt::Display for DurableSaveCommandErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "durable save refused: {self:?}")
    }
}

impl std::error::Error for DurableSaveCommandErrorV1 {}

/// Runs the selected restart-stable Save operation through Vellum's public
/// transport-neutral client. The presentation supplies correlation and target
/// facts only; authentication, action authority, persistence and receipt truth
/// remain owned by the connected Vellum service and its selected adapter.
pub fn run_durable_save_v1<T: ServicePort>(
    transport: T,
    command: DurableSaveCommandV1,
    operation: DurableOperationRefV1,
    document_ordinal: u64,
    expected_revision: u64,
) -> Result<DurableSaveReportV1, DurableSaveCommandErrorV1> {
    let mut client = Client::connect(transport).map_err(|_| DurableSaveCommandErrorV1::Connect)?;
    let inventory = match client.workspace_inventory() {
        Ok(ReplyV1::Accepted(vellum_sdk_client::protocol::OutcomeV1::WorkspaceInventory(
            inventory,
        ))) => inventory,
        Ok(ReplyV1::Refused(_)) | Ok(ReplyV1::Accepted(_)) => {
            disconnect_client(client)?;
            return Err(DurableSaveCommandErrorV1::InventoryRefused);
        }
        Err(_) => {
            disconnect_client(client)?;
            return Err(DurableSaveCommandErrorV1::InventoryAccess);
        }
    };
    let document = match inventory
        .documents
        .into_iter()
        .find(|document| document.get() == document_ordinal)
    {
        Some(document) => document,
        None => {
            disconnect_client(client)?;
            return Err(DurableSaveCommandErrorV1::DocumentAbsent);
        }
    };
    let intent = DurableEffectIntentV1::SaveDocument {
        document,
        expected_revision,
    };
    let outcome = match command {
        DurableSaveCommandV1::Execute => match client.execute_durable(operation, intent) {
            Ok(outcome) => map_execute(document, expected_revision, outcome),
            Err(ServiceAccessErrorV1::OutcomeUnknown) => {
                let _transport = client.into_transport();
                return Ok(DurableSaveReportV1 {
                    outcome: DurableSaveOutcomeV1::OutcomeUnknown,
                    disconnect: DurableSaveDisconnectV1::AbandonedAfterOutcomeUnknown,
                });
            }
            Err(_) => Err(DurableSaveCommandErrorV1::Access),
        },
        DurableSaveCommandV1::Lookup => match client.lookup_durable_receipt(operation, intent) {
            Ok(DurableReceiptLookupOutcomeV1::Complete(outcome)) => {
                map_execute(document, expected_revision, outcome)
            }
            Ok(DurableReceiptLookupOutcomeV1::RequestConflict) => {
                Ok(DurableSaveOutcomeV1::RequestConflict)
            }
            Ok(DurableReceiptLookupOutcomeV1::OutcomeUnknown) => {
                Ok(DurableSaveOutcomeV1::OutcomeUnknown)
            }
            Ok(DurableReceiptLookupOutcomeV1::AbsentOrForeign) => {
                Ok(DurableSaveOutcomeV1::AbsentOrForeign)
            }
            Ok(DurableReceiptLookupOutcomeV1::Unavailable) => Ok(DurableSaveOutcomeV1::Unavailable),
            Err(_) => Err(DurableSaveCommandErrorV1::Access),
        },
    };
    disconnect_client(client)?;
    let outcome = outcome?;
    Ok(DurableSaveReportV1 {
        outcome,
        disconnect: DurableSaveDisconnectV1::Clean,
    })
}

fn disconnect_client<T: ServicePort>(client: Client<T>) -> Result<(), DurableSaveCommandErrorV1> {
    client
        .disconnect()
        .map(|_transport| ())
        .map_err(|_| DurableSaveCommandErrorV1::Disconnect)
}

fn map_execute(
    document: WorkspaceDocumentIdV1,
    expected_revision: u64,
    outcome: DurableExecuteOutcomeV1,
) -> Result<DurableSaveOutcomeV1, DurableSaveCommandErrorV1> {
    match outcome {
        DurableExecuteOutcomeV1::Completed(DurableEffectReceiptV1::DocumentSaved(receipt))
            if receipt.document == document && receipt.source_revision == expected_revision =>
        {
            Ok(DurableSaveOutcomeV1::Completed {
                application: receipt.persistence.application,
                synchronization: receipt.persistence.synchronization,
                no_future_effects: receipt.persistence.no_future_effects,
                quiescent: receipt.persistence.quiescent,
                adapter_failure: receipt.persistence.adapter_failure,
                cleanup_failed: receipt.persistence.cleanup_failed,
                baseline_published: receipt.persistence.baseline_published,
                outcome_unknown: receipt.persistence.outcome_unknown,
            })
        }
        DurableExecuteOutcomeV1::Completed(_) => Err(DurableSaveCommandErrorV1::ReceiptMismatch),
        DurableExecuteOutcomeV1::Refused(_) => Ok(DurableSaveOutcomeV1::Refused),
        DurableExecuteOutcomeV1::Retired => Ok(DurableSaveOutcomeV1::Retired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_sdk_client::protocol::{
        ClientId, DurableExecuteRequestV1, DurableReceiptLookupV1, OutcomeV1, RequestV1,
        SaveReceiptV1, ServiceControlErrorV1, WorkspaceIdV1, WorkspaceInventoryV1,
    };

    struct Port {
        lookup: bool,
    }

    impl Port {
        fn document() -> WorkspaceDocumentIdV1 {
            WorkspaceDocumentIdV1::new(WorkspaceIdV1::new(7).unwrap(), 11).unwrap()
        }

        fn complete(request: DurableExecuteRequestV1) -> DurableExecuteOutcomeV1 {
            assert_eq!(request.operation.to_bytes(), [9; 16]);
            assert_eq!(
                request.intent,
                DurableEffectIntentV1::SaveDocument {
                    document: Self::document(),
                    expected_revision: 13,
                }
            );
            DurableExecuteOutcomeV1::Completed(DurableEffectReceiptV1::DocumentSaved(
                vellum_sdk_client::protocol::DurableSaveReceiptV1 {
                    document: Self::document(),
                    source_revision: 13,
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
    }

    impl ServicePort for Port {
        fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
            Ok(ClientId::new(3).unwrap())
        }

        fn execute(&mut self, _request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
            Ok(ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(
                WorkspaceInventoryV1 {
                    workspace: WorkspaceIdV1::new(7).unwrap(),
                    documents: vec![Self::document()],
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
            assert!(!self.lookup);
            Ok(Self::complete(request))
        }

        fn lookup_durable_receipt(
            &mut self,
            lookup: DurableReceiptLookupV1,
        ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
            assert!(self.lookup);
            Ok(DurableReceiptLookupOutcomeV1::Complete(Self::complete(
                DurableExecuteRequestV1 {
                    operation: lookup.operation,
                    client: lookup.client,
                    request: lookup.request,
                    intent: lookup.intent,
                },
            )))
        }
    }

    #[test]
    fn execute_and_lookup_share_the_public_client_guard_and_exact_intent() {
        for (command, lookup) in [
            (DurableSaveCommandV1::Execute, false),
            (DurableSaveCommandV1::Lookup, true),
        ] {
            let report = run_durable_save_v1(
                Port { lookup },
                command,
                DurableOperationRefV1::new([9; 16]).unwrap(),
                11,
                13,
            )
            .unwrap();
            assert!(matches!(
                report,
                DurableSaveReportV1 {
                    outcome: DurableSaveOutcomeV1::Completed {
                        application: SaveApplicationV1::Written,
                        synchronization: SaveSynchronizationV1::Succeeded,
                        no_future_effects: true,
                        quiescent: true,
                        adapter_failure: false,
                        cleanup_failed: false,
                        baseline_published: true,
                        outcome_unknown: false,
                    },
                    disconnect: DurableSaveDisconnectV1::Clean,
                }
            ));
        }
    }
}
