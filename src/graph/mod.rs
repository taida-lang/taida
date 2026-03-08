//! Structural Introspection — Graph model, extraction, query, and formatting.
//!
//! Taida's 10 operators and single-direction constraint make it possible to
//! deterministically extract graphs from the AST by syntax traversal alone.

pub mod extract;
pub mod format;
pub mod model;
pub mod query;
pub mod verify;
