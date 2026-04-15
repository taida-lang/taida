//! C12B-038: Type-tag propagation primitives.
//!
//! This module collects the **pure, state-independent** helpers that
//! participate in runtime type-tag propagation. Historically the full
//! tag-propagation state (HashSets of Bool-returning functions, the
//! `param_tag_vars` map, per-Expr inference) lived scattered through
//! `src/codegen/lower.rs` (7,489 lines, 69 call sites for the state
//! maps alone — see C12B-038 in `.dev/C12_BLOCKERS.md`). The C12 design
//! document's Workstream K planned a full tag-prop module; for C12 we
//! establish the module boundary with the pieces that *are* free of
//! `self: &Lowering` state, so that the `lower.rs` mechanical split
//! (C12B-024) can migrate the remaining state-dependent helpers in a
//! follow-up PR without needing to first invent the module.
//!
//! # Tag values
//!
//! Taida's runtime tags are seven 2-byte discriminators consumed by the
//! C runtime (`taida_io_stdout_with_tag`, `taida_pack_field_with_tag`,
//! etc). The wasm runtime shares the same encoding; see
//! `src/codegen/runtime_core_wasm/01_core.inc.c` for the switch that
//! reads these values.
//!
//! | Constant         | Value | Meaning                                    |
//! |------------------|-------|--------------------------------------------|
//! | `TAG_INT`        | 0     | `:Int`                                     |
//! | `TAG_FLOAT`      | 1     | `:Float`                                   |
//! | `TAG_BOOL`       | 2     | `:Bool`                                    |
//! | `TAG_STR`        | 3     | `:Str`                                     |
//! | `TAG_PACK`       | 4     | BuchiPack / TypeInst / Lax / Result / Async / HashMap / Set |
//! | `TAG_LIST`       | 5     | `:List[T]`                                 |
//! | `TAG_CLOSURE`    | 6     | `Lambda` / function-valued                 |
//! | `TAG_UNKNOWN`    | -1    | Cannot be determined at compile time       |
//!
//! These constants replace a handful of magic numbers in
//! `src/codegen/lower.rs` so that tag-prop readers can grep for a single
//! name instead of `0`/`1`/`2`.
//!
//! The magic-number call sites inside `lower.rs` are left as-is in this
//! commit to keep the diff minimal; subsequent work should swap the
//! literals for the named constants as the function-by-function
//! migration proceeds.

/// `:Int` runtime tag.
pub(crate) const TAG_INT: i64 = 0;
/// `:Float` runtime tag.
pub(crate) const TAG_FLOAT: i64 = 1;
/// `:Bool` runtime tag.
pub(crate) const TAG_BOOL: i64 = 2;
/// `:Str` runtime tag.
pub(crate) const TAG_STR: i64 = 3;
/// BuchiPack / TypeInst / Lax / Result / Async / HashMap / Set runtime tag.
pub(crate) const TAG_PACK: i64 = 4;
/// `:List[T]` runtime tag.
pub(crate) const TAG_LIST: i64 = 5;
/// Closure (lambda / function-valued) runtime tag.
pub(crate) const TAG_CLOSURE: i64 = 6;
/// "Cannot determine at compile time" sentinel. The runtime fall-backs
/// to polymorphic dispatch on this tag. Kept as a named constant even
/// though the main consumer (`Lowering::expr_type_tag`) currently
/// inlines the literal `-1`; swapping to the named constant is part of
/// the follow-up migration tracked in C12B-038.
#[allow(dead_code)]
pub(crate) const TAG_UNKNOWN: i64 = -1;

/// Convert a static [`crate::parser::TypeExpr`] annotation to the
/// corresponding runtime tag. This is a pure function: it depends only
/// on the annotation tree, not on any collected `Lowering` state.
///
/// Generic envelopes (`Lax`, `Gorillax`, `RelaxedGorillax`, `Result`,
/// `Async`, `HashMap`, `Set`) are all treated as [`TAG_PACK`] because
/// they are represented internally as typed BuchiPacks. Unknown generic
/// names fall back to [`TAG_INT`] to preserve pre-existing behaviour
/// (the fallback was previously hard-coded inside `lower.rs`).
///
/// Note that `TypeExpr::Named("Bytes")` also preserves the historical
/// fallback-to-pack behaviour here. Some runtime detection paths treat
/// concrete Bytes values as string-like, but the static annotation path
/// used by `type_expr_to_tag` never had a dedicated Bytes branch.
pub(crate) fn type_expr_to_tag(ty: &crate::parser::TypeExpr) -> i64 {
    use crate::parser::TypeExpr;
    match ty {
        TypeExpr::Named(n) => match n.as_str() {
            "Int" => TAG_INT,
            "Float" => TAG_FLOAT,
            "Bool" => TAG_BOOL,
            "Str" => TAG_STR,
            _ => TAG_PACK, // user-defined types are Packs
        },
        TypeExpr::List(_) => TAG_LIST,
        TypeExpr::BuchiPack(_) => TAG_PACK,
        TypeExpr::Function(_, _) => TAG_CLOSURE,
        TypeExpr::Generic(name, _) => match name.as_str() {
            "Lax" | "Gorillax" | "RelaxedGorillax" | "Result" | "Async" => TAG_PACK,
            "HashMap" => TAG_PACK,
            "Set" => TAG_PACK,
            _ => TAG_INT,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TypeExpr;

    #[test]
    fn named_primitives_map_to_expected_tags() {
        assert_eq!(type_expr_to_tag(&TypeExpr::Named("Int".into())), TAG_INT);
        assert_eq!(
            type_expr_to_tag(&TypeExpr::Named("Float".into())),
            TAG_FLOAT
        );
        assert_eq!(type_expr_to_tag(&TypeExpr::Named("Bool".into())), TAG_BOOL);
        assert_eq!(type_expr_to_tag(&TypeExpr::Named("Str".into())), TAG_STR);
    }

    #[test]
    fn named_user_type_is_pack() {
        assert_eq!(
            type_expr_to_tag(&TypeExpr::Named("MyRecord".into())),
            TAG_PACK
        );
    }

    #[test]
    fn named_bytes_preserves_historical_pack_fallback() {
        assert_eq!(type_expr_to_tag(&TypeExpr::Named("Bytes".into())), TAG_PACK);
    }

    #[test]
    fn list_maps_to_tag_list() {
        let list_of_int = TypeExpr::List(Box::new(TypeExpr::Named("Int".into())));
        assert_eq!(type_expr_to_tag(&list_of_int), TAG_LIST);
    }

    #[test]
    fn pack_envelopes_map_to_pack() {
        let lax = TypeExpr::Generic("Lax".into(), vec![TypeExpr::Named("Str".into())]);
        let result = TypeExpr::Generic("Result".into(), vec![TypeExpr::Named("Int".into())]);
        let async_ = TypeExpr::Generic("Async".into(), vec![TypeExpr::Named("Int".into())]);
        let hashmap = TypeExpr::Generic(
            "HashMap".into(),
            vec![TypeExpr::Named("Str".into()), TypeExpr::Named("Int".into())],
        );
        let set = TypeExpr::Generic("Set".into(), vec![TypeExpr::Named("Int".into())]);
        assert_eq!(type_expr_to_tag(&lax), TAG_PACK);
        assert_eq!(type_expr_to_tag(&result), TAG_PACK);
        assert_eq!(type_expr_to_tag(&async_), TAG_PACK);
        assert_eq!(type_expr_to_tag(&hashmap), TAG_PACK);
        assert_eq!(type_expr_to_tag(&set), TAG_PACK);
    }

    #[test]
    fn unknown_generic_falls_back_to_int() {
        let unknown = TypeExpr::Generic("Foo".into(), vec![TypeExpr::Named("Int".into())]);
        // Pre-C12 behaviour preserved: fallback is `TAG_INT` (0), not
        // `TAG_UNKNOWN`. A future refactor may tighten this.
        assert_eq!(type_expr_to_tag(&unknown), TAG_INT);
    }

    #[test]
    fn function_type_is_closure_tag() {
        let f = TypeExpr::Function(
            vec![TypeExpr::Named("Int".into())],
            Box::new(TypeExpr::Named("Bool".into())),
        );
        assert_eq!(type_expr_to_tag(&f), TAG_CLOSURE);
    }

    #[test]
    fn buchi_pack_type_is_pack_tag() {
        // The tag is independent of the pack's field list, so an empty
        // `Vec<FieldDef>` is sufficient to exercise the branch without
        // importing the full `FieldDef`/`Span` constructors.
        let p = TypeExpr::BuchiPack(vec![]);
        assert_eq!(type_expr_to_tag(&p), TAG_PACK);
    }

    #[test]
    fn tag_unknown_sentinel_is_negative() {
        // `TAG_UNKNOWN` is the caller-visible sentinel for "can't determine
        // at compile time"; no `TypeExpr` ever resolves to it (it only
        // appears in `expr_type_tag` which is still in `lower.rs` and
        // depends on `Lowering` state). We still pin its value here so
        // that any future refactor consolidating the dispatch table
        // keeps the encoding stable.
        assert_eq!(TAG_UNKNOWN, -1);
        const _: () = assert!(TAG_UNKNOWN < TAG_INT);
    }
}
