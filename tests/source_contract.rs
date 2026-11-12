use serde_json::Value;
use std::{fs, path::Path};

const VELLUM_REVISION: &str = "f4a53ba664acf2a12b6fadfd7d378eef63307c39";
const RECOVERY_REVISION: &str = "920dbfccf3806378fd0274cc51fadffac405931b";
const KEYSTONE_REVISION: &str = "3486252cdbaeb2d784756076f43ef6157ed0b7f8";

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
    assert_eq!(source["remoteReachabilityQualified"], false);
    assert_eq!(source["cratesIoPublished"], false);

    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("manifest");
    assert!(manifest.contains("repository = \"https://github.com/gantbd/vellum-tui\""));
    assert!(manifest.contains("publish = false"));
    assert!(manifest.contains("path = \"../vellum/host/vellum-editor\""));
    assert!(manifest.contains("vellum-sdk-client"));
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
    assert!(workflow.contains("ref: f4a53ba664acf2a12b6fadfd7d378eef63307c39"));
    assert!(workflow.contains("ref: 3486252cdbaeb2d784756076f43ef6157ed0b7f8"));
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
    assert!(journey.contains("--shutdown"));
    assert!(!journey.contains("cargo publish"));
    assert!(!journey.contains("git push"));

    let test = fs::read_to_string(root.join("tests/live_service.rs")).expect("live test");
    assert!(test.contains("vellum_sdk_client"));
    assert!(test.contains("ServiceEditor"));
    assert!(!test.contains("vellum_daemon"));
}
