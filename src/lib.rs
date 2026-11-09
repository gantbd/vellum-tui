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
