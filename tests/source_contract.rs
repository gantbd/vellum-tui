use serde_json::Value;
use std::{fs, path::Path};

const VELLUM_REVISION: &str = "418f4eeb5522309d9245b6d455085bac5cd1f8cd";
const RECOVERY_REVISION: &str = "920dbfccf3806378fd0274cc51fadffac405931b";
const KEYSTONE_REVISION: &str = "8a7970cb77e668f3488ecd97dae7c9e765f170cc";

#[test]
fn unpublished_peer_is_bound_to_the_recovered_and_current_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Value = serde_json::from_str(
        &fs::read_to_string(root.join("release/source.json")).expect("source contract"),
    )
    .expect("canonical source JSON");
    assert_eq!(source["schema"], 1);
    assert_eq!(source["package"], "vellum-tui");
    assert_eq!(source["vellumRevision"], VELLUM_REVISION);
    assert_eq!(source["recoveredFromVellumRevision"], RECOVERY_REVISION);
    assert_eq!(source["keystoneRevision"], KEYSTONE_REVISION);
    assert_eq!(source["remoteReachabilityQualified"], true);
    assert_eq!(source["cratesIoPublished"], false);

    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("manifest");
    assert!(manifest.contains("repository = \"https://github.com/gantbd/vellum-tui\""));
    assert!(manifest.contains("publish = false"));
    assert!(manifest.contains("path = \"../vellum/host/vellum-editor\""));
    assert!(manifest.contains("vellum-sdk-client"));
    for forbidden in [
        "vellum-clipboard-macos",
        "vellum-adapter-local-fs",
        "vellum-adapter-local-authentication",
        "vellum-engine",
    ] {
        let dependencies = manifest
            .split("[dev-dependencies]")
            .next()
            .expect("normal dependency section");
        assert!(
            !dependencies.contains(forbidden),
            "library imports {forbidden}"
        );
    }
    let cli_manifest = fs::read_to_string(root.join("cli/Cargo.toml")).expect("CLI manifest");
    assert!(cli_manifest.contains("name = \"vellum-tui-cli\""));
    assert!(cli_manifest.contains("vellum-tui = { path = \"..\" }"));
    for forbidden in ["vellum-client =", "vellum-transport =", "vellum-protocol ="] {
        assert!(
            !manifest.contains(forbidden),
            "terminal peer bypasses the SDK through {forbidden}"
        );
    }
    assert!(!manifest.contains("vellum-iced"));
    assert!(!manifest.contains("vellum-gui-macos"));
}

#[test]
fn payload_has_no_links_or_inbound_application_dependency() {
    fn inspect(path: &Path, root: &Path) {
        for entry in fs::read_dir(path).expect("payload directory") {
            let entry = entry.expect("payload entry");
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("contained path");
            if relative.starts_with("target") || relative.starts_with(".git") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).expect("payload metadata");
            assert!(!metadata.file_type().is_symlink(), "link: {relative:?}");
            if metadata.is_dir() {
                inspect(&path, root);
            } else {
                assert!(metadata.is_file(), "non-file: {relative:?}");
                if relative.starts_with("src")
                    && path.extension().is_some_and(|extension| extension == "rs")
                {
                    let source = fs::read_to_string(&path).expect("Rust source");
                    for forbidden in ["vellum_daemon", "vellum_mcp", "vellum_gui_macos"] {
                        assert!(
                            !source.contains(forbidden),
                            "{relative:?} imports {forbidden}"
                        );
                    }
                }
            }
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    inspect(root, root);
}

#[test]
fn github_ci_is_read_only_and_exactly_coordinates_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("CI workflow");
    assert_eq!(
        workflow
            .matches("actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683")
            .count(),
        3
    );
    assert!(workflow.contains("ref: 418f4eeb5522309d9245b6d455085bac5cd1f8cd"));
    assert!(workflow.contains("ref: 8a7970cb77e668f3488ecd97dae7c9e765f170cc"));
    assert!(workflow.contains("permissions:\n  contents: read"));
    assert!(workflow.contains("./tests/live/service_journey.sh"));
    for forbidden in [
        "upload-artifact",
        "cargo publish",
        "git push",
        "release create",
    ] {
        assert!(!workflow.contains(forbidden), "CI contains {forbidden}");
    }
}

#[test]
fn live_service_journey_uses_only_the_peer_sdk_and_real_daemon_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let journey = fs::read_to_string(root.join("tests/live/service_journey.sh"))
        .expect("live service journey");
    assert!(journey.contains("--package vellum-daemon"));
    assert!(journey.contains("--allow-edit --allow-save"));
    assert!(journey.contains("--test live_service"));
    assert!(journey.contains("indeterminate exact replay"));
    assert!(journey.contains("headless --service"));
    assert!(journey.contains("--shutdown"));
    assert!(!journey.contains("cargo publish"));
    assert!(!journey.contains("git push"));

    let test = fs::read_to_string(root.join("tests/live_service.rs")).expect("live test");
    assert!(test.contains("vellum_sdk_client"));
    assert!(test.contains("ServiceEditor"));
    assert!(test.contains("LoseFirstSaveReply"));
    assert!(test.contains("PersistenceProjectionV1::Indeterminate"));
    assert!(test.contains("uncertain save must replay exactly"));
    assert!(!test.contains("vellum_daemon"));
}

#[test]
fn standalone_cli_has_explicit_headless_service_mode() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let entry = fs::read_to_string(root.join("cli/src/main.rs")).expect("standalone entry point");
    assert!(entry.contains("headless --service SOCKET"));
    assert!(entry.contains("if headless"));
    assert!(entry.contains("refresh_inventory"));
    assert!(entry.contains("serde_json::to_string_pretty"));
    assert!(entry.contains("vellum_tui::CAPABILITIES"));
    assert!(entry.contains("disconnect_outcome_unknown"));
    assert!(
        fs::read_to_string(root.join("src/lib.rs"))
            .unwrap()
            .contains("workspace.inventory.v1")
    );
    let capabilities = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(capabilities.contains("workspace.edit.v1"));
    assert!(capabilities.contains("workspace.save.v1"));
    assert!(entry.contains("ratatui::run"));

    let presentation = fs::read_to_string(root.join("src/lib.rs")).expect("presentation library");
    assert!(!presentation.contains("args_os"));
    assert!(!presentation.contains("ratatui::run"));
    assert!(!root.join("src/main.rs").exists());
}
