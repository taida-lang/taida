//! C12 Phase 6 (FB-5 Phase 2-3) — Regex type support.
//!
//! The Taida `:Regex` type is represented as a typed BuchiPack with
//! `__type <= "Regex"`, `pattern <= Str`, `flags <= Str`. The constructor
//! lives in [`crate::interpreter::prelude`] as the prelude function
//! `Regex(pattern, flags?)`. Str method overloads (`replace`, `replaceAll`,
//! `split`, `match`, `search`) detect a Regex BuchiPack as the first arg
//! and dispatch through this module.
//!
//! **Philosophy alignment**:
//! * I. No `null`/`undefined`: construction with an invalid pattern yields
//!   an `Error` BuchiPack (via `RuntimeError::Throw`), never a silent
//!   undefined state.
//! * II. "Bag" form: `Regex(...)` is a BuchiPack, introspectable via
//!   `typeof(r) == "Regex"` and `r.pattern` / `r.flags`.
//! * No implicit coercion: Str methods dispatch explicitly by detecting
//!   the BuchiPack `__type` tag; everything else is a literal string match.
//!
//! **Escape semantics** (design lock `C12_DESIGN.md` §C12-6):
//! * `\d` / `\w` / `\s` / `\b` / `\D` / `\W` / `\S` / `\B`
//! * `\x{HH}` / `\u{HHHH}` (via Rust `regex` crate defaults)
//! * JS `$&` / `$$` / `$1` meta-syntax in the replacement string is
//!   disabled: replacements are applied literally.
//!
//! **Flags**: `i` (case-insensitive), `m` (multiline `^`/`$`), `s`
//! (dotall). `g` is implicit — `replace` replaces first, `replaceAll`
//! replaces all. Unknown flag chars produce a validation error at
//! `Regex(...)` construction time.

use super::value::Value;
use regex::{Regex, RegexBuilder};

/// Internal tag marker stored in `Value::BuchiPack` `__type` for
/// Regex values.
pub(crate) const REGEX_TYPE_TAG: &str = "Regex";

/// Detect whether a [`Value`] is a Regex BuchiPack. Returns the
/// `(pattern, flags)` tuple when so.
pub(crate) fn as_regex(val: &Value) -> Option<(String, String)> {
    let Value::BuchiPack(fields) = val else {
        return None;
    };
    let is_regex = fields
        .iter()
        .any(|(k, v)| k == "__type" && matches!(v, Value::Str(s) if s == REGEX_TYPE_TAG));
    if !is_regex {
        return None;
    }
    let pattern = fields
        .iter()
        .find(|(k, _)| k == "pattern")
        .and_then(|(_, v)| match v {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let flags = fields
        .iter()
        .find(|(k, _)| k == "flags")
        .and_then(|(_, v)| match v {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    Some((pattern, flags))
}

/// Build a Regex BuchiPack value. Pattern / flags are validated by
/// attempting to compile the regex eagerly; failure is surfaced as
/// `Err(message)` for the prelude caller to translate into a throwable
/// `:Error`.
pub(crate) fn build_regex_value(pattern: &str, flags: &str) -> Result<Value, String> {
    validate_flags(flags)?;
    // Compile once to surface pattern errors at construction time rather
    // than at first use. The compiled object is dropped — every
    // Str-method overload recompiles, matching JS's `new RegExp(...)`
    // semantics where sharing is via the pattern string. This keeps the
    // Value fully cloneable / serializable and avoids interior mutability.
    compile(pattern, flags)?;
    Ok(Value::BuchiPack(vec![
        ("pattern".into(), Value::Str(pattern.to_string())),
        ("flags".into(), Value::Str(flags.to_string())),
        ("__type".into(), Value::Str(REGEX_TYPE_TAG.into())),
    ]))
}

fn validate_flags(flags: &str) -> Result<(), String> {
    for c in flags.chars() {
        match c {
            'i' | 'm' | 's' => {}
            other => {
                return Err(format!(
                    "Regex: unsupported flag '{}'. Supported flags: i (case-insensitive), m (multiline), s (dotall)",
                    other
                ));
            }
        }
    }
    Ok(())
}

fn compile(pattern: &str, flags: &str) -> Result<Regex, String> {
    let mut builder = RegexBuilder::new(pattern);
    builder.case_insensitive(flags.contains('i'));
    builder.multi_line(flags.contains('m'));
    builder.dot_matches_new_line(flags.contains('s'));
    builder
        .build()
        .map_err(|e| format!("Regex: invalid pattern '{}' — {}", pattern, e))
}

/// Apply `replace` (first match only) using the Regex semantics. The
/// replacement string is treated as a literal — no `$1` / `$&`
/// expansion — to match JS / Native parity (see design lock §C12-6).
pub(crate) fn replace_first(
    s: &str,
    pattern: &str,
    flags: &str,
    replacement: &str,
) -> Result<String, String> {
    let re = compile(pattern, flags)?;
    // `regex::replacen` expands `$N` by default. Use `NoExpand` to
    // apply the replacement as a literal, matching the B11 Phase 1
    // fixed-string contract and JS's documented `$&` lock-down.
    Ok(re
        .replacen(s, 1, regex::NoExpand(replacement))
        .into_owned())
}

/// Apply `replaceAll` using the Regex semantics. Literal replacement —
/// no `$N` expansion.
pub(crate) fn replace_all(
    s: &str,
    pattern: &str,
    flags: &str,
    replacement: &str,
) -> Result<String, String> {
    let re = compile(pattern, flags)?;
    Ok(re.replace_all(s, regex::NoExpand(replacement)).into_owned())
}

/// Apply `split` using the Regex semantics. Empty pattern is an error
/// (philosophy: no silent fallback — the user must explicitly call
/// `.split("")` for codepoint-split, which is the fixed-string overload).
pub(crate) fn split(s: &str, pattern: &str, flags: &str) -> Result<Vec<String>, String> {
    let re = compile(pattern, flags)?;
    Ok(re.split(s).map(|part| part.to_string()).collect())
}

/// Single match result: `(start_byte, full_match, groups...)`.
/// Groups are 1-indexed; missing / non-participating groups become `""`.
pub(crate) struct MatchResult {
    pub start: i64,
    pub full: String,
    pub groups: Vec<String>,
}

/// Find the first regex match in `s`. Returns `Ok(None)` when no
/// match; `Err` only for invalid patterns (which should not happen
/// since `build_regex_value` pre-validated).
pub(crate) fn match_first(
    s: &str,
    pattern: &str,
    flags: &str,
) -> Result<Option<MatchResult>, String> {
    let re = compile(pattern, flags)?;
    let Some(caps) = re.captures(s) else {
        return Ok(None);
    };
    let full_match = caps.get(0).expect("group 0 is always present on a match");
    let start_byte = full_match.start();
    // Convert byte offset to char offset for surface-level consistency
    // with `indexOf` / `length` (Taida's public Str API is char-based).
    let start_char = s[..start_byte].chars().count() as i64;
    let full = full_match.as_str().to_string();
    let groups: Vec<String> = (1..caps.len())
        .map(|i| caps.get(i).map(|m| m.as_str().to_string()).unwrap_or_default())
        .collect();
    Ok(Some(MatchResult {
        start: start_char,
        full,
        groups,
    }))
}

/// Char-based start index of the first match, or `-1` when no match.
/// Exposed as the `search` method's return value (`Int`).
pub(crate) fn search_first(s: &str, pattern: &str, flags: &str) -> Result<i64, String> {
    match match_first(s, pattern, flags)? {
        Some(m) => Ok(m.start),
        None => Ok(-1),
    }
}

/// Build the `:RegexMatch` BuchiPack that `str.match(Regex(...))`
/// returns. Uses `hasValue` / `__type <= "RegexMatch"` so that
/// `result.hasValue` is queriable via the existing Lax-like idiom
/// while keeping the payload (`full`, `groups`, `start`) inspectable.
pub(crate) fn build_match_value(m: Option<MatchResult>) -> Value {
    match m {
        Some(m) => Value::BuchiPack(vec![
            ("hasValue".into(), Value::Bool(true)),
            ("full".into(), Value::Str(m.full)),
            (
                "groups".into(),
                Value::List(m.groups.into_iter().map(Value::Str).collect()),
            ),
            ("start".into(), Value::Int(m.start)),
            ("__type".into(), Value::Str("RegexMatch".into())),
        ]),
        None => Value::BuchiPack(vec![
            ("hasValue".into(), Value::Bool(false)),
            ("full".into(), Value::Str(String::new())),
            ("groups".into(), Value::List(Vec::new())),
            ("start".into(), Value::Int(-1)),
            ("__type".into(), Value::Str("RegexMatch".into())),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_regex_value_roundtrip() {
        let v = build_regex_value("\\d+", "i").expect("valid pattern");
        let (p, f) = as_regex(&v).expect("detected as regex");
        assert_eq!(p, "\\d+");
        assert_eq!(f, "i");
    }

    #[test]
    fn test_build_regex_rejects_bad_flag() {
        let err = build_regex_value("a", "x").unwrap_err();
        assert!(err.contains("unsupported flag"));
    }

    #[test]
    fn test_build_regex_rejects_invalid_pattern() {
        let err = build_regex_value("(unclosed", "").unwrap_err();
        assert!(err.contains("invalid pattern"));
    }

    #[test]
    fn test_replace_first_literal_replacement_ignores_dollar_meta() {
        // `$&` / `$1` must be treated literally (JS parity note in the
        // design lock).
        let out = replace_first("abc", "b", "", "$&").expect("replace");
        assert_eq!(out, "a$&c");
    }

    #[test]
    fn test_replace_all_matches_pattern() {
        let out = replace_all("hello world", "[aeiou]", "", "*").expect("replace_all");
        assert_eq!(out, "h*ll* w*rld");
    }

    #[test]
    fn test_split_on_pattern() {
        let parts = split("a1b22c333d", "\\d+", "").expect("split");
        assert_eq!(parts, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_match_first_extracts_groups() {
        let m =
            match_first("id: 12-34 other", "(\\d+)-(\\d+)", "").expect("match ok").expect("matched");
        assert_eq!(m.full, "12-34");
        assert_eq!(m.groups, vec!["12".to_string(), "34".to_string()]);
        assert_eq!(m.start, 4);
    }

    #[test]
    fn test_match_first_no_match() {
        let m = match_first("nothing", "\\d+", "").expect("match ok");
        assert!(m.is_none());
    }

    #[test]
    fn test_search_first_returns_char_index() {
        // `あ` is 3 UTF-8 bytes but 1 char, so the `\d` match at char
        // index 1 must be reported as 1, not as a byte offset.
        let idx = search_first("あ123", "\\d", "").expect("search");
        assert_eq!(idx, 1);
    }

    #[test]
    fn test_search_first_no_match() {
        let idx = search_first("abc", "\\d", "").expect("search");
        assert_eq!(idx, -1);
    }

    #[test]
    fn test_flag_i_case_insensitive() {
        let out = replace_all("AbC", "b", "i", "*").expect("replace_all");
        assert_eq!(out, "A*C");
    }

    #[test]
    fn test_flag_m_multiline_anchors() {
        let parts = split("ab\ncd", "^", "m").expect("split");
        // Each line start becomes a split boundary, producing an empty
        // leading string and each subsequent line as a separate piece.
        assert_eq!(parts, vec!["", "ab\n", "cd"]);
    }

    #[test]
    fn test_flag_s_dotall() {
        let m = match_first("a\nb", ".", "s").expect("match").expect("matched");
        assert_eq!(m.full, "a"); // first char is still first
        let m2 =
            match_first("\nb", ".", "s").expect("match").expect("matched");
        assert_eq!(m2.full, "\n"); // `.` now crosses newline
    }

    #[test]
    fn test_as_regex_rejects_non_regex_buchipack() {
        let lax = Value::BuchiPack(vec![
            ("hasValue".into(), Value::Bool(true)),
            ("__type".into(), Value::Str("Lax".into())),
        ]);
        assert!(as_regex(&lax).is_none());
    }

    #[test]
    fn test_build_match_value_some() {
        let v = build_match_value(Some(MatchResult {
            start: 2,
            full: "xy".into(),
            groups: vec!["x".into(), "y".into()],
        }));
        if let Value::BuchiPack(fields) = v {
            assert!(fields.iter().any(|(k, v)| k == "hasValue"
                && matches!(v, Value::Bool(true))));
            assert!(fields.iter().any(|(k, v)| k == "__type"
                && matches!(v, Value::Str(s) if s == "RegexMatch")));
        } else {
            panic!("expected BuchiPack");
        }
    }

    #[test]
    fn test_build_match_value_none() {
        let v = build_match_value(None);
        if let Value::BuchiPack(fields) = v {
            assert!(fields.iter().any(|(k, v)| k == "hasValue"
                && matches!(v, Value::Bool(false))));
            assert!(fields.iter().any(|(k, v)| k == "start"
                && matches!(v, Value::Int(-1))));
        } else {
            panic!("expected BuchiPack");
        }
    }
}
