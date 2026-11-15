//! Reusable Ratatui presentation for Vellum.
//!
//! This library owns bounded projection and input translation only. Concrete
//! filesystem, clipboard, Unicode, authentication, terminal lifecycle, and
//! process choices belong to a standalone binary or another composition root.
#![forbid(unsafe_code)]

mod open_prompt;
pub use open_prompt::*;
mod terminal;
pub use terminal::*;
mod comments;
pub use comments::*;
mod service;
pub use service::*;

pub use vellum_editor::EditorFrame;
