//! Structural Introspection — AI-oriented graph analysis and verification.
//!
//! `taida graph` outputs AI-oriented unified JSON for codebase comprehension.
//! `taida verify` runs structural verification checks on Taida source files.

pub mod ai_format;
pub mod verify;

// Internal modules used by verify — not part of the public API.
pub(crate) mod extract;
pub(crate) mod model;
pub(crate) mod query;

/// Escape special characters for JSON strings (RFC 8259 compliant).
pub(crate) fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}
