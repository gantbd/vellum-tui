//! Concrete local reference composition selected by the standalone CLI.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootAdmissionError {
    Empty,
    Plan(vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1),
}

impl std::fmt::Display for RootAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace root admission refused: {self:?}")
    }
}

impl std::error::Error for RootAdmissionError {}

pub fn plan_workspace_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<Vec<PathBuf>, RootAdmissionError> {
    use vellum_adapter_local_fs::{
        DirectoryDiscoveryLimits, WorkspaceRootPlanLimitsV1, WorkspaceRootPlanOutcomeV1,
    };
    let limits = WorkspaceRootPlanLimitsV1 {
        max_roots: 16,
        discovery: DirectoryDiscoveryLimits {
            max_entries: 4096,
            max_files: 16,
            max_depth: 64,
            max_path_bytes: 4096,
            max_total_path_bytes: 65_536,
        },
    };
    match vellum_adapter_local_fs::plan_workspace_roots_v1(roots, limits) {
        WorkspaceRootPlanOutcomeV1::Complete { files, .. } if files.is_empty() => {
            Err(RootAdmissionError::Empty)
        }
        WorkspaceRootPlanOutcomeV1::Complete { files, .. } => Ok(files),
        WorkspaceRootPlanOutcomeV1::Refused { reason, .. } => Err(RootAdmissionError::Plan(reason)),
    }
}

pub fn reference_session_config() -> vellum_editor::ReferenceSessionConfig {
    use vellum_adapter_local_fs::{FilesystemLimits, LocalFilesystemFactory};
    let mut config = vellum_editor::reference_profile(
        LocalFilesystemFactory::new(FilesystemLimits {
            max_resources: 16,
            max_bytes: 65_536,
            max_stage_attempts: 16,
            max_metadata_bytes: 65_536,
        })
        .erase(),
        Box::new(vellum_adapter_annotation_json::JsonAnnotationCodec),
    );
    config.engine.invocation = Some(vellum_engine::coordinator::InvocationLimits {
        max_lifetime_issued: 1_000_000,
        max_retained: 8,
        max_intent_bytes: 262_144,
    });
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vellum-tui-root-admission-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_state_accepts_no_synthetic_document() {
        assert_eq!(
            plan_workspace_roots(std::iter::empty::<&Path>()),
            Err(RootAdmissionError::Empty)
        );
        let fixture = Fixture::new();
        assert_eq!(
            plan_workspace_roots([&fixture.0]),
            Err(RootAdmissionError::Empty)
        );
    }

    #[test]
    fn directory_plan_is_sorted_duplicate_free_and_file_compatible() {
        let fixture = Fixture::new();
        let nested = fixture.0.join("nested");
        fs::create_dir(&nested).unwrap();
        let first = fixture.0.join("b.txt");
        let second = nested.join("a.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();

        let plan = plan_workspace_roots([fixture.0.as_path(), first.as_path()]).unwrap();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(plan, expected);
    }

    #[test]
    fn symbolic_links_refuse_the_complete_plan() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source.txt");
        fs::write(&source, "source").unwrap();
        symlink(&source, fixture.0.join("link.txt")).unwrap();

        assert!(matches!(
            plan_workspace_roots([&fixture.0]),
            Err(RootAdmissionError::Plan(
                vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1::Discovery(
                    vellum_adapter_local_fs::DirectoryDiscoveryRefusal::UnsupportedResource
                )
            ))
        ));
    }

    #[test]
    fn aggregate_plan_refuses_more_than_the_editor_capacity() {
        let fixture = Fixture::new();
        let mut roots = Vec::new();
        for index in 0..17 {
            let path = fixture.0.join(format!("{index:02}.txt"));
            fs::write(&path, index.to_string()).unwrap();
            roots.push(path);
        }
        assert_eq!(
            plan_workspace_roots(&roots),
            Err(RootAdmissionError::Plan(
                vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1::RootLimit
            ))
        );
    }
}
