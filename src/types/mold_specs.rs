//! Builtin mold specification registry.
//!
//! This module centralizes builtin mold metadata. Codegen uses the
//! return-kind portion for runtime type tags, and the type checker can
//! use the same entry for arity and option validation. The shape is
//! intentionally data-first: adding a builtin mold should mean adding
//! or updating one registry row, not copying name lists across backends.
//!
//! Older lowering code treated every `MoldInst` as Pack-tagged, which
//! made runtime display dispatch misclassify Str-returning molds such as
//! `Upper`, `Lower`, `Trim`, and `Join`. Keeping the return tag here lets
//! callers choose the correct runtime display path directly.
//!
//! Tag values match the `TAIDA_TAG_*` constants in `native_runtime.c`
//! and the `WASM_TAG_*` constants in `runtime_core_wasm.c`:
//!
//! - `0` — Int
//! - `1` — Float
//! - `2` — Bool
//! - `3` — Str
//! - `4` — Pack
//! - `5` — List
//! - `6` — Closure
//!
//! A mold whose return tag depends on its arguments (e.g. `Slice`,
//! `Concat`, `Abs`, `Map`, `Filter`, `If`) returns `MoldReturnKind::Dynamic`
//! and callers fall back to argument-based inference or the UNKNOWN tag
//! (`-1`). User-defined molds (not in this table) always return Pack.
//!
/// Known return-type kind for a builtin mold.
///
/// `Dynamic` marks molds whose return type is determined by argument
/// types (e.g. `Concat`, `Slice`, `Abs`, `Map`). Such cases are resolved
/// at each call site; this table only fixes the cases that are constant
/// regardless of arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoldReturnKind {
    Int,
    Float,
    Bool,
    Str,
    Pack,
    List,
    /// Return type depends on arguments; caller must infer.
    Dynamic,
}

/// Coarse checker-side kind for a positional `[]` argument or option value.
///
/// These kinds deliberately stay small. They describe public mold contracts
/// that are stable across backends; richer return-type inference still lives
/// in the checker because it can depend on the actual argument expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoldArgKind {
    Any,
    Bool,
    Function,
    List,
    ListOrStream,
    Numeric,
}

/// Named `()` option accepted by a builtin mold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoldOptionSpec {
    pub name: &'static str,
    pub kind: MoldArgKind,
}

/// Builtin mold metadata shared by return-tag propagation and checker
/// front-door validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoldSpec {
    pub name: &'static str,
    pub arity_min: usize,
    pub arity_max: Option<usize>,
    pub arg_kinds: &'static [MoldArgKind],
    pub return_kind: MoldReturnKind,
    pub options: &'static [MoldOptionSpec],
    /// Some legacy molds still need permissive arity until their runtime
    /// contracts are audited. `true` means the checker should enforce the
    /// registry arity now.
    pub checker_enforced: bool,
}

impl MoldSpec {
    pub const fn exact(
        name: &'static str,
        arity: usize,
        arg_kinds: &'static [MoldArgKind],
        return_kind: MoldReturnKind,
    ) -> Self {
        Self {
            name,
            arity_min: arity,
            arity_max: Some(arity),
            arg_kinds,
            return_kind,
            options: &[],
            checker_enforced: false,
        }
    }

    pub const fn range(
        name: &'static str,
        min: usize,
        max: Option<usize>,
        arg_kinds: &'static [MoldArgKind],
        return_kind: MoldReturnKind,
    ) -> Self {
        Self {
            name,
            arity_min: min,
            arity_max: max,
            arg_kinds,
            return_kind,
            options: &[],
            checker_enforced: false,
        }
    }

    pub const fn enforce_checker(mut self) -> Self {
        self.checker_enforced = true;
        self
    }

    pub const fn with_options(mut self, options: &'static [MoldOptionSpec]) -> Self {
        self.options = options;
        self
    }

    pub fn accepts_arity(&self, arity: usize) -> bool {
        arity >= self.arity_min && self.arity_max.is_none_or(|max| arity <= max)
    }

    pub fn arity_description(&self) -> String {
        match (self.arity_min, self.arity_max) {
            (min, Some(max)) if min == max => min.to_string(),
            (min, Some(max)) => format!("{}-{}", min, max),
            (min, None) => format!("at least {}", min),
        }
    }
}

impl MoldReturnKind {
    /// Return the runtime type tag for this kind, or `None` if the kind
    /// is Dynamic (argument-dependent).
    pub fn tag(self) -> Option<i64> {
        match self {
            MoldReturnKind::Int => Some(0),
            MoldReturnKind::Float => Some(1),
            MoldReturnKind::Bool => Some(2),
            MoldReturnKind::Str => Some(3),
            MoldReturnKind::Pack => Some(4),
            MoldReturnKind::List => Some(5),
            MoldReturnKind::Dynamic => None,
        }
    }
}

const ANY1: &[MoldArgKind] = &[MoldArgKind::Any];
const ANY2: &[MoldArgKind] = &[MoldArgKind::Any, MoldArgKind::Any];
const ANY3: &[MoldArgKind] = &[MoldArgKind::Any, MoldArgKind::Any, MoldArgKind::Any];
const BOOL_ANY_ANY: &[MoldArgKind] = &[MoldArgKind::Bool, MoldArgKind::Any, MoldArgKind::Any];
const LIST1: &[MoldArgKind] = &[MoldArgKind::List];
const LIST_FUNCTION: &[MoldArgKind] = &[MoldArgKind::ListOrStream, MoldArgKind::Function];
const LIST_ANY: &[MoldArgKind] = &[MoldArgKind::List, MoldArgKind::Any];
const LIST_ANY_FUNCTION: &[MoldArgKind] =
    &[MoldArgKind::List, MoldArgKind::Any, MoldArgKind::Function];
const LIST_FUNCTION_ONLY: &[MoldArgKind] = &[MoldArgKind::List, MoldArgKind::Function];
const ASYNC_NUM: &[MoldArgKind] = &[MoldArgKind::Any, MoldArgKind::Numeric];

const SORT_OPTIONS: &[MoldOptionSpec] = &[
    MoldOptionSpec {
        name: "reverse",
        kind: MoldArgKind::Bool,
    },
    MoldOptionSpec {
        name: "desc",
        kind: MoldArgKind::Bool,
    },
    MoldOptionSpec {
        name: "by",
        kind: MoldArgKind::Function,
    },
];
const UNIQUE_OPTIONS: &[MoldOptionSpec] = &[MoldOptionSpec {
    name: "by",
    kind: MoldArgKind::Function,
}];

pub static MOLD_SPECS: &[MoldSpec] = &[
    // Primitive conversion / wrappers.
    MoldSpec::exact("Int", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Float", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Bool", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Str", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Bytes", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("UInt8", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Char", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("CodePoint", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Utf8Encode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Utf8Decode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U16BE", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U16LE", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U32BE", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U32LE", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U16BEDecode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U16LEDecode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U32BEDecode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("U32LEDecode", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::range("Lax", 0, Some(1), ANY1, MoldReturnKind::Pack),
    MoldSpec::range("Result", 1, Some(2), ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("Async", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Gorillax", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("RelaxedGorillax", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("Stream", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("StreamFrom", 1, LIST1, MoldReturnKind::Pack),
    MoldSpec::exact("Molten", 0, &[], MoldReturnKind::Pack),
    MoldSpec::exact("Cage", 2, ANY2, MoldReturnKind::Pack),
    // Numeric / math.
    MoldSpec::exact("Sqrt", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Pow", 2, ANY2, MoldReturnKind::Float),
    MoldSpec::range("Log", 1, Some(2), ANY2, MoldReturnKind::Float),
    MoldSpec::exact("Exp", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Ln", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Log2", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Log10", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Sin", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Cos", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Tan", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Asin", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Acos", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Atan", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Atan2", 2, ANY2, MoldReturnKind::Float),
    MoldSpec::exact("Sinh", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Cosh", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Tanh", 1, ANY1, MoldReturnKind::Float),
    MoldSpec::exact("Floor", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("Ceil", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("Round", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("Truncate", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("Abs", 1, ANY1, MoldReturnKind::Dynamic),
    MoldSpec::exact("Clamp", 3, ANY3, MoldReturnKind::Dynamic),
    MoldSpec::exact("Div", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("Mod", 2, ANY2, MoldReturnKind::Pack),
    // String / bytes.
    MoldSpec::exact("TypeName", 1, ANY1, MoldReturnKind::Str),
    MoldSpec::exact("Upper", 1, ANY1, MoldReturnKind::Str),
    MoldSpec::exact("Lower", 1, ANY1, MoldReturnKind::Str),
    MoldSpec::exact("Trim", 1, ANY1, MoldReturnKind::Str),
    MoldSpec::exact("Replace", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("ReplaceAll", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("Repeat", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("Pad", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("PadLeft", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("PadRight", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("Join", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("ToFixed", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("ToRadix", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("StrOf", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("ByteSlice", 3, ANY3, MoldReturnKind::Str),
    MoldSpec::exact("StringRepeatJoin", 2, ANY2, MoldReturnKind::Str),
    MoldSpec::exact("ByteLength", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("ByteAt", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("CharAt", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("ByteSet", 3, ANY3, MoldReturnKind::Pack),
    MoldSpec::exact("BytesToList", 1, ANY1, MoldReturnKind::List),
    MoldSpec::range("Slice", 2, Some(3), ANY3, MoldReturnKind::Dynamic),
    MoldSpec::range("Concat", 2, None, ANY2, MoldReturnKind::Dynamic),
    MoldSpec::exact("Chars", 1, ANY1, MoldReturnKind::List),
    MoldSpec::exact("Split", 2, ANY2, MoldReturnKind::List),
    // Bool / type predicates.
    MoldSpec::exact("TypeIs", 2, ANY2, MoldReturnKind::Bool),
    MoldSpec::exact("TypeExtends", 2, ANY2, MoldReturnKind::Bool),
    MoldSpec::exact("Exists", 1, ANY1, MoldReturnKind::Bool),
    MoldSpec::exact("Contains", 2, ANY2, MoldReturnKind::Bool),
    MoldSpec::exact("SpanEquals", 3, ANY3, MoldReturnKind::Bool),
    MoldSpec::exact("SpanStartsWith", 3, ANY3, MoldReturnKind::Bool),
    MoldSpec::exact("SpanContains", 3, ANY3, MoldReturnKind::Bool),
    MoldSpec::exact("SpanSlice", 3, ANY3, MoldReturnKind::Pack),
    // Bit / ordinal.
    MoldSpec::exact("BitAnd", 2, ANY2, MoldReturnKind::Int),
    MoldSpec::exact("BitOr", 2, ANY2, MoldReturnKind::Int),
    MoldSpec::exact("BitXor", 2, ANY2, MoldReturnKind::Int),
    MoldSpec::exact("BitNot", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::exact("ShiftL", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("ShiftR", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("ShiftRU", 2, ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("Ordinal", 1, ANY1, MoldReturnKind::Int),
    // Collections and HOF molds.
    MoldSpec::exact("Length", 1, ANY1, MoldReturnKind::Int),
    MoldSpec::range("Count", 1, Some(2), ANY2, MoldReturnKind::Int),
    MoldSpec::range("Find", 1, Some(2), ANY2, MoldReturnKind::Pack),
    MoldSpec::range("FindIndex", 1, Some(2), ANY2, MoldReturnKind::Int),
    MoldSpec::range("FindIndexLax", 1, Some(2), ANY2, MoldReturnKind::Pack),
    MoldSpec::exact("IndexOf", 2, ANY2, MoldReturnKind::Int),
    MoldSpec::exact("LastIndexOf", 2, ANY2, MoldReturnKind::Int),
    MoldSpec::exact("Sort", 1, LIST1, MoldReturnKind::List)
        .with_options(SORT_OPTIONS)
        .enforce_checker(),
    MoldSpec::exact("Unique", 1, LIST1, MoldReturnKind::List)
        .with_options(UNIQUE_OPTIONS)
        .enforce_checker(),
    MoldSpec::exact("Flatten", 1, LIST1, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Reverse", 1, ANY1, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Take", 2, LIST_ANY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("TakeWhile", 2, LIST_FUNCTION_ONLY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Drop", 2, LIST_ANY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("DropWhile", 2, LIST_FUNCTION_ONLY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Append", 2, LIST_ANY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Prepend", 2, LIST_ANY, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Zip", 2, ANY2, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Enumerate", 1, LIST1, MoldReturnKind::List).enforce_checker(),
    MoldSpec::exact("Map", 2, LIST_FUNCTION, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Filter", 2, LIST_FUNCTION, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Fold", 3, LIST_ANY_FUNCTION, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Foldr", 3, LIST_ANY_FUNCTION, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Reduce", 3, LIST_ANY_FUNCTION, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Sum", 1, LIST1, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Min", 1, LIST1, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("Max", 1, LIST1, MoldReturnKind::Dynamic).enforce_checker(),
    MoldSpec::exact("If", 3, BOOL_ANY_ANY, MoldReturnKind::Dynamic).enforce_checker(),
    // Async combinators.
    MoldSpec::exact("Cancel", 1, ANY1, MoldReturnKind::Pack),
    MoldSpec::exact("All", 1, LIST1, MoldReturnKind::Pack),
    MoldSpec::exact("Race", 1, LIST1, MoldReturnKind::Pack),
    MoldSpec::exact("Timeout", 2, ASYNC_NUM, MoldReturnKind::Pack),
];

/// Look up a builtin mold specification by name.
pub fn lookup_mold_spec(name: &str) -> Option<&'static MoldSpec> {
    MOLD_SPECS.iter().find(|spec| spec.name == name)
}

/// Look up the return-type kind of a builtin mold by name.
///
/// Returns `None` for user-defined molds (they always return `Pack` in
/// codegen — callers should treat `None` as "unknown builtin" and apply
/// their usual fallback).
pub fn lookup_mold_return_kind(name: &str) -> Option<MoldReturnKind> {
    lookup_mold_spec(name).map(|spec| spec.return_kind)
}

/// Return the runtime tag for a builtin mold, if the return kind is
/// statically known (not Dynamic, not user-defined).
///
/// Used by `src/codegen/lower.rs::expr_type_tag()` to decide whether a
/// `MoldInst` can be dispatched through `taida_io_stdout_with_tag`
/// directly (no `convert_to_string` fallback needed).
pub fn mold_return_tag(name: &str) -> Option<i64> {
    lookup_mold_return_kind(name).and_then(|k| k.tag())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_returning_molds_map_to_str_tag() {
        // `Str[x]()` returns Lax[Str], not a bare Str.
        for name in [
            "Upper",
            "Lower",
            "Trim",
            "Replace",
            "ReplaceAll",
            "Repeat",
            "Pad",
            "PadLeft",
            "PadRight",
            "Join",
            "ToFixed",
            "ToRadix",
            // Byte-level slice + single-alloc repeat/join.
            "ByteSlice",
            "StringRepeatJoin",
            "TypeName",
        ] {
            assert_eq!(mold_return_tag(name), Some(3), "{name} should be Str (3)");
        }
    }

    #[test]
    fn bool_returning_molds_map_to_bool_tag() {
        // `Bool[x]()` returns Lax[Bool], not a bare Bool.
        for name in ["TypeIs", "TypeExtends", "Exists", "Contains"] {
            assert_eq!(mold_return_tag(name), Some(2), "{name} should be Bool (2)");
        }
    }

    #[test]
    fn span_aware_molds_map_to_expected_tags() {
        // Span-aware comparison molds return Bool; `SpanSlice` returns a
        // Pack sub-span `@(start, len)`.
        for name in ["SpanEquals", "SpanStartsWith", "SpanContains"] {
            assert_eq!(mold_return_tag(name), Some(2), "{name} should be Bool (2)");
        }
        assert_eq!(
            mold_return_tag("SpanSlice"),
            Some(4),
            "SpanSlice should be Pack (4) — returns @(start, len) sub-span"
        );
        // `StrOf[span, raw]()` materializes a span pack into an owned Str.
        assert_eq!(
            mold_return_tag("StrOf"),
            Some(3),
            "StrOf should be Str (3) — returns owned materialized string"
        );
    }

    #[test]
    fn int_returning_molds_map_to_int_tag() {
        // `Int[x]()` returns Lax[Int], not a bare Int.
        for name in [
            "Length",
            "Count",
            "IndexOf",
            "LastIndexOf",
            "FindIndex",
            "Floor",
            "Ceil",
            "Round",
            "Truncate",
            "BitAnd",
            "BitOr",
            "BitXor",
            "BitNot",
            // `ByteLength` returns bare Int (bytes in UTF-8 encoding).
            "ByteLength",
        ] {
            assert_eq!(mold_return_tag(name), Some(0), "{name} should be Int (0)");
        }
    }

    #[test]
    fn primitive_conversion_molds_map_to_pack_tag() {
        // Primitive conversion molds return Lax (= Pack), not their
        // primitive output type directly.
        for name in ["Int", "Float", "Bool", "Str"] {
            assert_eq!(
                mold_return_tag(name),
                Some(4),
                "{name}[x]() returns Lax (= Pack tag 4) — not the primitive output type"
            );
        }
    }

    #[test]
    fn list_returning_molds_map_to_list_tag() {
        for name in [
            "Chars",
            "Split",
            "Sort",
            "Unique",
            "Flatten",
            "Take",
            "TakeWhile",
            "Drop",
            "DropWhile",
            "Append",
            "Prepend",
            "Zip",
            "Enumerate",
            "BytesToList",
        ] {
            assert_eq!(mold_return_tag(name), Some(5), "{name} should be List (5)");
        }
    }

    #[test]
    fn dynamic_molds_return_none_for_tag() {
        for name in [
            "Map", "Filter", "Reverse", "Concat", "Slice", "Abs", "Clamp", "Sum", "Min", "Max",
            "If", "Fold", "Foldr", "Reduce",
        ] {
            assert_eq!(mold_return_tag(name), None, "{name} is argument-dependent");
        }
    }

    #[test]
    fn user_defined_molds_return_none() {
        assert_eq!(mold_return_tag("MyMold"), None);
        assert_eq!(mold_return_tag("UserPack"), None);
    }

    #[test]
    fn pack_returning_molds_map_to_pack_tag() {
        for name in [
            "Lax",
            "Result",
            "Async",
            "Gorillax",
            "Stream",
            "StreamFrom",
            "Molten",
            "Cage",
            "Find",
            "CharAt",
            "ShiftL",
            "ShiftR",
            "ByteSet",
            // `ByteAt` returns Lax[Int] (Pack at runtime).
            "ByteAt",
        ] {
            assert_eq!(mold_return_tag(name), Some(4), "{name} should be Pack (4)");
        }
    }
}
