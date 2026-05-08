//! Typed HIR / Typed AST infrastructure (E34 Phase 1.2, Lock-B=C foundation).
//!
//! Phase 1 では、untyped AST を再構築せずに **side table** として expr → Type の
//! mapping を保持する最小実装を提供する。codegen lower (Phase 2) はこの side table
//! を consume し、`expr_is_bool` 等の判定で `typed_expr_table[expr_id].type == Bool`
//! の query だけを行う。`bool_vars` / `bool_returning_funcs` / allow-list / `infer_type_name`
//! 等の codegen 内型推論機構は Phase 2 で完全削除される。
//!
//! ## Lock-B=C 文 (`.dev/E34_DESIGN.md::Phase 0 Locked Decisions::Lock-B`)
//!
//! > `expr_is_bool` を codegen の allow-list / method-name 推測から完全に追放する。
//! > type-checker / generic solver が確定した型を Typed HIR / Typed AST の expression
//! > type table に書き込み、codegen は `typed_expr_table[expr_id].type == Bool`
//! > だけを真とする。
//!
//! ## Phase 1 設計上の trade-off
//!
//! - **`ExprId` は span-based hash**: parser を改修しない。`(start, end, discriminant_tag)`
//!   の 3-tuple をキーにする。同一 program 内で span は unique なので衝突は起こらない。
//! - **side table 方式 (= AST mirror ではない)**: Lock-B=C 文の理想形は AST mirror
//!   (Typed AST) だが、Phase 1 では HashMap で機能等価。Phase 3+ で必要なら拡張。
//! - **`record(&Expr, Type)` は idempotent**: 同じ Expr に対して再呼び出しすると
//!   後勝ちで上書き。Phase 1.4 の Lambda bidirectional 推論で hint 付き path が
//!   override する場面を許容する。
//!
//! ## Phase 1 acceptance
//!
//! - `tests/typed_hir_smoke.rs` で table の埋まり方を検証
//! - `has_residual_unknown` で Lock-C 文「type-checker 完了後の Typed HIR には
//!   `Type::Unknown` 残らない」(対象 fixture) を assert

use std::collections::HashMap;

use crate::parser::Expr;

use super::types::Type;

/// Stable identifier of an `Expr` in the AST.
///
/// Phase 1: parser を改修せず、span + discriminant tag の 3-tuple をキーにする。
/// 同一 program 内で span は unique なので、(start, end, discriminant_tag) で
/// expression を一意に識別できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId {
    start: usize,
    end: usize,
    discriminant: u8,
}

impl ExprId {
    /// Compute a stable id from an expression's span + variant discriminant.
    pub fn from_expr(expr: &Expr) -> Self {
        let span = expr.span();
        Self {
            start: span.start,
            end: span.end,
            discriminant: expr_discriminant(expr),
        }
    }
}

/// Phase 1: minimal Typed HIR. AST → Type の side table。
///
/// codegen lower (Phase 2) はこの table を consume し、`is_bool` / `lookup`
/// で typed query を行う。Phase 1 では誰も読まないが、acceptance test で
/// table の record 状態を verify する。
#[derive(Debug, Clone, Default)]
pub struct TypedExprTable {
    types: HashMap<ExprId, Type>,
}

impl TypedExprTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the inferred type of `expr`. Idempotent: same expr → same type.
    /// Phase 1.4 の Lambda bidirectional 推論で hint-付き path が override
    /// するケースを許容する (後勝ち)。
    pub fn record(&mut self, expr: &Expr, ty: Type) {
        let id = ExprId::from_expr(expr);
        self.types.insert(id, ty);
    }

    /// Lookup the type of `expr`. Returns `None` if not recorded.
    pub fn lookup(&self, expr: &Expr) -> Option<&Type> {
        let id = ExprId::from_expr(expr);
        self.types.get(&id)
    }

    /// Phase 2 codegen 用 convenience: typed bool query。
    /// `expr_is_bool` を `table.is_bool(&expr)` に置換するための API。
    pub fn is_bool(&self, expr: &Expr) -> bool {
        matches!(self.lookup(expr), Some(Type::Bool))
    }

    /// Number of recorded entries. Used for diagnostics / introspection.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// True if no entries recorded.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Phase 1 acceptance helper: returns true if any recorded type contains a
    /// `Type::Unknown` (residual after type-checker completes).
    ///
    /// Lock-C 文: 「`Type::Unknown` は推論途中の変数のみ許可、type-checker
    /// 完了後の signature には残さない」。
    /// `tests/typed_hir_smoke.rs` で fixture ごとに assert する。
    pub fn has_residual_unknown(&self) -> bool {
        self.types.values().any(|t| t.contains_concrete_unknown())
    }

    /// Iterate over all recorded `(ExprId, Type)` pairs. Phase 2 codegen 用。
    pub fn iter(&self) -> impl Iterator<Item = (&ExprId, &Type)> {
        self.types.iter()
    }
}

/// Discriminant tag for each `Expr` variant. Used in `ExprId` to disambiguate
/// expressions that happen to share a `Span` (rare but theoretically possible
/// when parser nests expressions).
fn expr_discriminant(expr: &Expr) -> u8 {
    match expr {
        Expr::IntLit(_, _) => 1,
        Expr::FloatLit(_, _) => 2,
        Expr::StringLit(_, _) => 3,
        Expr::TemplateLit(_, _) => 4,
        Expr::BoolLit(_, _) => 5,
        Expr::Gorilla(_) => 6,
        Expr::Ident(_, _) => 7,
        Expr::Placeholder(_) => 8,
        Expr::Hole(_) => 9,
        Expr::BuchiPack(_, _) => 10,
        Expr::ListLit(_, _) => 11,
        Expr::BinaryOp(_, _, _, _) => 12,
        Expr::UnaryOp(_, _, _) => 13,
        Expr::FuncCall(_, _, _) => 14,
        Expr::MethodCall(_, _, _, _) => 15,
        Expr::FieldAccess(_, _, _) => 16,
        Expr::CondBranch(_, _) => 17,
        Expr::Pipeline(_, _) => 18,
        Expr::MoldInst(_, _, _, _) => 19,
        Expr::Unmold(_, _) => 20,
        Expr::Lambda(_, _, _) => 21,
        Expr::TypeInst(_, _, _) => 22,
        Expr::EnumVariant(_, _, _) => 23,
        Expr::TypeLiteral(_, _, _) => 24,
        Expr::Throw(_, _) => 25,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;

    fn span(start: usize, end: usize) -> Span {
        Span::new(start, end, 1, 1)
    }

    #[test]
    fn record_and_lookup_basic() {
        let mut table = TypedExprTable::new();
        let expr = Expr::IntLit(42, span(0, 2));
        assert!(table.is_empty());
        table.record(&expr, Type::Int);
        assert_eq!(table.len(), 1);
        assert_eq!(table.lookup(&expr), Some(&Type::Int));
    }

    #[test]
    fn record_idempotent_overwrites_with_last_write() {
        let mut table = TypedExprTable::new();
        let expr = Expr::IntLit(42, span(0, 2));
        table.record(&expr, Type::Unknown);
        table.record(&expr, Type::Int);
        // Last-write wins (Phase 1.4 hint-付き path 上書き対応)
        assert_eq!(table.lookup(&expr), Some(&Type::Int));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn is_bool_query() {
        let mut table = TypedExprTable::new();
        let bool_expr = Expr::BoolLit(true, span(0, 4));
        let int_expr = Expr::IntLit(42, span(5, 7));
        table.record(&bool_expr, Type::Bool);
        table.record(&int_expr, Type::Int);
        assert!(table.is_bool(&bool_expr));
        assert!(!table.is_bool(&int_expr));
    }

    #[test]
    fn has_residual_unknown_detects_unknown() {
        let mut table = TypedExprTable::new();
        let e1 = Expr::IntLit(1, span(0, 1));
        let e2 = Expr::IntLit(2, span(2, 3));
        table.record(&e1, Type::Int);
        assert!(!table.has_residual_unknown());
        table.record(&e2, Type::Unknown);
        assert!(table.has_residual_unknown());
    }

    #[test]
    fn has_residual_unknown_detects_nested_unknown() {
        let mut table = TypedExprTable::new();
        let e = Expr::ListLit(vec![], span(0, 2));
        table.record(&e, Type::List(Box::new(Type::Unknown)));
        assert!(table.has_residual_unknown());
    }

    #[test]
    fn discriminant_disambiguates_same_span() {
        // 2 expressions with the same span (theoretical edge case) get
        // different ids via discriminant.
        let mut table = TypedExprTable::new();
        let int_expr = Expr::IntLit(0, span(0, 1));
        let bool_expr = Expr::BoolLit(true, span(0, 1));
        table.record(&int_expr, Type::Int);
        table.record(&bool_expr, Type::Bool);
        assert_eq!(table.lookup(&int_expr), Some(&Type::Int));
        assert_eq!(table.lookup(&bool_expr), Some(&Type::Bool));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn lookup_returns_none_for_unrecorded() {
        let table = TypedExprTable::new();
        let expr = Expr::IntLit(42, span(0, 2));
        assert!(table.lookup(&expr).is_none());
        assert!(!table.is_bool(&expr));
    }

    #[test]
    fn iter_yields_all_recorded() {
        let mut table = TypedExprTable::new();
        let e1 = Expr::IntLit(1, span(0, 1));
        let e2 = Expr::StringLit("x".to_string(), span(2, 5));
        table.record(&e1, Type::Int);
        table.record(&e2, Type::Str);
        let entries: Vec<_> = table.iter().collect();
        assert_eq!(entries.len(), 2);
    }
}
