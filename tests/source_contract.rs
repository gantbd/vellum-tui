use serde_json::Value;
use std::{fs, path::Path};

const VELLUM_REVISION: &str = "4de24b3b75b4ec712cbe6e5f0a0f7b80a2967a19";
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
