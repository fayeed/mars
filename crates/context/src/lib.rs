//! Project-aware context capture and content-addressed artifact storage.

mod artifacts;
mod capture;

pub use capture::{CaptureError, ContextItem, ContextKind, ContextService};
