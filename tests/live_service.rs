#![cfg(target_os = "macos")]

use std::{path::PathBuf, time::Duration};
use vellum_sdk_client::{
    protocol::{
        CommandV1, ReplyV1, RequestV1, ServiceAccessErrorV1, ServiceControlErrorV1, ServicePort,
    },
    transport::{RemoteTransport, TransportLimits, UnixConnector},
    workspace::{PersistenceProjectionV1, WorkspaceClientErrorV1, WorkspaceClientV1},
};
use vellum_tui::{ServiceAction, ServiceEditor};

struct LoseFirstSaveReply<T> {
    inner: T,
    lost: Option<RequestV1>,
    replayed: bool,
}

impl<T: ServicePort> ServicePort for LoseFirstSaveReply<T> {
    fn connect(&mut self) -> Result<vellum_sdk_client::protocol::ClientId, ServiceControlErrorV1> {
        self.inner.connect()
    }

    fn execute(&mut self, request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
        if matches!(request.command, CommandV1::Save { .. }) {
            if let Some(lost) = &self.lost {
                assert_eq!(lost, &request, "uncertain save must replay exactly");
                self.replayed = true;
                return self.inner.execute(request);
            }
            let reply = self.inner.execute(request.clone())?;
            self.lost = Some(request);
            let _ = reply;
            return Err(ServiceAccessErrorV1::OutcomeUnknown);
        }
        self.inner.execute(request)
    }

    fn disconnect(
        &mut self,
        client: vellum_sdk_client::protocol::ClientId,
    ) -> Result<(), ServiceControlErrorV1> {
        self.inner.disconnect(client)
    }
}

#[test]
#[ignore = "started by tests/live/service_journey.sh against a real daemon"]
fn real_daemon_open_edit_save_disconnect_journey() {
    let socket = PathBuf::from(
        std::env::var_os("VELLUM_LIVE_SOCKET").expect("live harness supplies socket"),
    );
    let connector = UnixConnector::with_timeout(socket, Duration::from_secs(2)).unwrap();
    let transport = RemoteTransport::new(connector, TransportLimits::reference()).unwrap();
    let workspace = WorkspaceClientV1::connect(transport).unwrap();
    let mut editor = ServiceEditor::open(workspace).unwrap();

    let initial = editor.frame(8).expect("startup document projected");
    assert_eq!(initial.lines, ["alpha"]);
    assert!(!initial.dirty);
    assert!(editor.source_is_current_and_clean());

    editor.apply(ServiceAction::Insert("X".to_owned())).unwrap();
    assert!(!editor.source_is_current_and_clean());
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Dirty
    );
    assert_eq!(editor.frame(8).unwrap().lines, ["Xalpha"]);

    editor.apply(ServiceAction::Save).unwrap();
    assert!(editor.source_is_current_and_clean());
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Clean
    );
    let transport = editor.disconnect().expect("exact disconnect");

    let workspace = WorkspaceClientV1::connect(LoseFirstSaveReply {
        inner: transport,
        lost: None,
        replayed: false,
    })
    .unwrap();
    let mut editor = ServiceEditor::open(workspace).unwrap();
    editor.apply(ServiceAction::Insert("Y".to_owned())).unwrap();
    assert_eq!(
        editor.apply(ServiceAction::Save),
        Err(vellum_tui::ServiceEditorError(
            WorkspaceClientErrorV1::Access(ServiceAccessErrorV1::OutcomeUnknown)
        ))
    );
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Indeterminate
    );
    assert!(!editor.source_is_current_and_clean());
    editor.apply(ServiceAction::Save).unwrap();
    assert!(editor.source_is_current_and_clean());
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Clean
    );
    let transport = editor.disconnect().expect("exact replay disconnect");
    assert!(transport.replayed);
}
