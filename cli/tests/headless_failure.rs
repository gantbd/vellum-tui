#![cfg(target_os = "macos")]

use std::process::Command;

#[test]
fn headless_connection_failure_is_versioned_json() {
    let socket = std::env::temp_dir().join(format!(
        "vellum-tui-missing-{}-service.sock",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_vellum-tui"))
        .arg("headless")
        .arg("--service")
        .arg(socket)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["mode"], "headless");
    assert_eq!(report["complete"], false);
    assert_eq!(report["peer"], "vellum");
    assert_eq!(report["capability"], "workspace.inventory.v1");
    assert_eq!(report["status"], "connect_refused");
}

#[test]
fn headless_configuration_failure_is_versioned_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_vellum-tui"))
        .arg("headless")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["mode"], "headless");
    assert_eq!(report["complete"], false);
    assert_eq!(report["peer"], "vellum");
    assert_eq!(report["capability"], "workspace.inventory.v1");
    assert_eq!(report["status"], "configuration_refused");
}
