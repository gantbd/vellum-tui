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

/// Presentation capabilities owned by this peer. They describe composition
/// support only; edit/save authority remains with the connected Vellum service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityManifest {
    pub schema: u8,
    pub peer: &'static str,
    pub interactive: &'static [&'static str],
    pub headless: &'static [&'static str],
}

pub const CAPABILITIES: CapabilityManifest = CapabilityManifest {
    schema: 1,
    peer: "vellum",
    interactive: &["workspace.inventory.v1"],
    headless: &["workspace.inventory.v1"],
};
