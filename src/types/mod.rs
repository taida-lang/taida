#![allow(clippy::module_inception)]

mod checker;
pub mod mold_returns;
mod types;
pub mod typed_hir;

pub use checker::*;
pub use typed_hir::{ExprId, TypedExprTable};
pub use types::*;
