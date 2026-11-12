//! Ratatui-only key translation and terminal projection. No source/history owner.
#![forbid(unsafe_code)]
#[cfg(target_os = "macos")]
mod open_prompt;
#[cfg(target_os = "macos")]
pub use open_prompt::*;
#[cfg(target_os = "macos")]
mod terminal;
#[cfg(target_os = "macos")]
pub use terminal::*;
#[cfg(target_os = "macos")]
mod comments;
#[cfg(target_os = "macos")]
pub use comments::*;

#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

/// Closed admission failures for one bounded set of file or directory roots.
/// Paths are deliberately absent from `Debug` and `Display` so callers can
/// report failures without accidentally disclosing workspace membership.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootAdmissionError {
    Empty,
    Capacity,
    Discovery(vellum_host::workspace::DirectoryDiscoveryRefusal),
}

#[cfg(target_os = "macos")]
impl std::fmt::Display for RootAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace root admission refused: {self:?}")
    }
}

#[cfg(target_os = "macos")]
impl std::error::Error for RootAdmissionError {}

/// Expand regular-file and directory roots through the qualified local adapter.
/// The returned plan is deterministic, duplicate-free, and capped by the
/// reference editor's document capacity. Locators remain subject to the
/// editor host's independent revalidation when they are opened.
#[cfg(target_os = "macos")]
pub fn plan_workspace_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<Vec<PathBuf>, RootAdmissionError> {
    use vellum_host::workspace::{DirectoryDiscoveryLimits, DirectoryDiscoveryOutcome};

    const MAX_FILES: usize = 16;
    let limits = DirectoryDiscoveryLimits {
        max_entries: 4096,
        max_files: MAX_FILES,
        max_depth: 64,
        max_path_bytes: 4096,
        max_total_path_bytes: 65_536,
    };
    let mut files = Vec::new();
    files
        .try_reserve_exact(MAX_FILES)
        .map_err(|_| RootAdmissionError::Capacity)?;
    let mut saw_root = false;
    for root in roots {
        saw_root = true;
        let discovered = vellum_adapter_local_fs::discover_workspace_files(root.as_ref(), limits);
        let discovered = match discovered {
            DirectoryDiscoveryOutcome::Complete { files, .. } => files,
            DirectoryDiscoveryOutcome::Refused { reason, .. } => {
                return Err(RootAdmissionError::Discovery(reason));
            }
        };
        for file in discovered {
            if files.contains(&file) {
                continue;
            }
            if files.len() == MAX_FILES {
                return Err(RootAdmissionError::Capacity);
            }
            files.push(file);
        }
    }
    if !saw_root || files.is_empty() {
        return Err(RootAdmissionError::Empty);
    }
    files.sort();
    Ok(files)
}

#[cfg(target_os = "macos")]
pub fn reference_session_config() -> vellum_model::ReferenceSessionConfig {
    use vellum_adapter_local_fs::{FilesystemLimits, LocalFilesystemFactory};
    let mut config = vellum_model::reference_profile(
        LocalFilesystemFactory::new(FilesystemLimits {
            max_resources: 16,
            max_bytes: 65536,
            max_stage_attempts: 16,
            max_metadata_bytes: 65536,
        })
        .erase(),
        Box::new(vellum_adapter_annotation_json::JsonAnnotationCodec),
    );
    config.engine.invocation = Some(vellum_engine::coordinator::InvocationLimits {
        max_lifetime_issued: 1_000_000,
        max_retained: 8,
        max_intent_bytes: 262144,
    });
    config
}

#[cfg(all(test, target_os = "macos"))]
mod root_admission_tests {
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
            Err(RootAdmissionError::Discovery(
                vellum_host::workspace::DirectoryDiscoveryRefusal::UnsupportedResource
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
            Err(RootAdmissionError::Capacity)
        );
    }
}
