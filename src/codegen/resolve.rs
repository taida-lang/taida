use crate::parser::*;
/// 変数解決パス — AST を走査して変数参照を解決する。
///
/// Phase N2: 各変数に一意の ID を割り当て、スコープを追跡する。
/// クロージャのキャプチャ変数も特定する。
use std::collections::HashMap;

/// 解決済み変数情報
#[derive(Debug, Clone)]
pub struct ResolvedVar {
    pub id: u32,
    pub name: String,
    pub scope_depth: usize,
}

/// 関数情報
#[derive(Debug, Clone)]
pub struct ResolvedFunc {
    pub id: u32,
    pub name: String,
    pub params: Vec<String>,
    pub captures: Vec<u32>, // キャプチャされた変数 ID
}

/// 変数解決の結果
#[derive(Debug)]
pub struct ResolveResult {
    /// 変数名 → 解決済み変数（現在のスコープ内で有効な最新の束縛）
    pub variables: HashMap<String, Vec<ResolvedVar>>,
    /// 関数名 → 関数情報
    pub functions: HashMap<String, ResolvedFunc>,
    pub next_var_id: u32,
    pub next_func_id: u32,
}

pub struct Resolver {
    scopes: Vec<HashMap<String, u32>>,
    variables: HashMap<u32, ResolvedVar>,
    functions: HashMap<String, ResolvedFunc>,
    next_var_id: u32,
    next_func_id: u32,
}

#[derive(Debug)]
pub struct ResolveError {
    pub message: String,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Resolve error: {}", self.message)
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()], // グローバルスコープ
            variables: HashMap::new(),
            functions: HashMap::new(),
            next_var_id: 0,
            next_func_id: 0,
        }
    }

    /// 変数を定義して ID を返す
    pub fn define_var(&mut self, name: &str) -> u32 {
        let id = self.next_var_id;
        self.next_var_id += 1;

        let depth = self.scopes.len() - 1;
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);

        self.variables.insert(
            id,
            ResolvedVar {
                id,
                name: name.to_string(),
                scope_depth: depth,
            },
        );

        id
    }

    /// 変数を参照して ID を返す
    pub fn lookup_var(&self, name: &str) -> Option<u32> {
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// 関数を定義
    pub fn define_func(&mut self, name: &str, params: &[String]) -> u32 {
        let id = self.next_func_id;
        self.next_func_id += 1;

        self.functions.insert(
            name.to_string(),
            ResolvedFunc {
                id,
                name: name.to_string(),
                params: params.to_vec(),
                captures: Vec::new(),
            },
        );

        // 関数名自体も変数として定義
        self.define_var(name);

        id
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub fn scope_depth(&self) -> usize {
        self.scopes.len() - 1
    }

    /// Program を走査して変数解決
    pub fn resolve_program(&mut self, program: &Program) -> Result<(), ResolveError> {
        for stmt in &program.statements {
            self.resolve_statement(stmt)?;
        }
        Ok(())
    }

    fn resolve_statement(&mut self, stmt: &Statement) -> Result<(), ResolveError> {
        match stmt {
            Statement::Assignment(assign) => {
                self.resolve_expr(&assign.value)?;
                self.define_var(&assign.target);
                Ok(())
            }
            Statement::FuncDef(func_def) => {
                let params: Vec<String> = func_def.params.iter().map(|p| p.name.clone()).collect();
                self.define_func(&func_def.name, &params);

                // 関数本体のスコープ
                self.push_scope();
                for param in &func_def.params {
                    self.define_var(&param.name);
                }
                for body_stmt in &func_def.body {
                    self.resolve_statement(body_stmt)?;
                }
                self.pop_scope();
                Ok(())
            }
            Statement::Expr(expr) => {
                self.resolve_expr(expr)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) -> Result<(), ResolveError> {
        match expr {
            Expr::Ident(_, _) => Ok(()),
            Expr::FuncCall(callee, args, _) => {
                self.resolve_expr(callee)?;
                for arg in args {
                    self.resolve_expr(arg)?;
                }
                Ok(())
            }
            Expr::BinaryOp(lhs, _, rhs, _) => {
                self.resolve_expr(lhs)?;
                self.resolve_expr(rhs)?;
                Ok(())
            }
            Expr::UnaryOp(_, operand, _) => {
                self.resolve_expr(operand)?;
                Ok(())
            }
            Expr::Pipeline(exprs, _) => {
                for expr in exprs {
                    self.resolve_expr(expr)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
