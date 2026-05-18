//! Runtime support modules for the `Value::Str` rope path.
//!
//! `Value::Str` uses interior wrapping with a gap-buffer rope path and
//! transparent promotion at the 1024-byte concat threshold.

pub mod str_rope;
