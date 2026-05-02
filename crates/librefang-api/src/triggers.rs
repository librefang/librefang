//! Re-exports of kernel trigger types used by API routes.
//!
//! Issue #3744: keep route modules from importing `librefang_kernel::triggers::*`
//! directly so the kernel surface area consumed by the API layer is centralized.

pub use librefang_kernel::triggers::{Trigger, TriggerId, TriggerPatch, TriggerPattern};
