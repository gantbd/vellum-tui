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
