#![cfg(target_os = "macos")]

use std::{path::PathBuf, time::Duration};
use vellum_sdk_client::{
    transport::{RemoteTransport, TransportLimits, UnixConnector},
    workspace::{PersistenceProjectionV1, WorkspaceClientV1},
};
use vellum_tui::{ServiceAction, ServiceEditor};

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

    editor.apply(ServiceAction::Insert("X".to_owned())).unwrap();
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Dirty
    );
    assert_eq!(editor.frame(8).unwrap().lines, ["Xalpha"]);

    editor.apply(ServiceAction::Save).unwrap();
    assert_eq!(
        editor.workspace().projection().persistence(),
        PersistenceProjectionV1::Clean
    );
    editor.disconnect().expect("exact disconnect");
}
