//! Taida IR — AST と CLIF IR の間に位置する中間表現。

/// IR 内の仮想変数（SSA 値への参照）
pub type IrVar = u32;

/// IR 命令
#[derive(Debug, Clone)]
pub enum IrInst {
    // ── リテラル ──
    ConstInt(IrVar, i64),
    ConstFloat(IrVar, f64),
    ConstStr(IrVar, String),
    ConstBool(IrVar, bool),

    // ── 変数操作（Phase N2） ──
    /// 変数定義: `name <= value` → DefVar(name, source_var)
    DefVar(String, IrVar),
    /// 変数参照: UseVar(dst, name)
    UseVar(IrVar, String),

    // ── ぶちパック操作（Phase N3） ──
    /// ぶちパック生成: PackNew(dst, field_count)
    PackNew(IrVar, usize),
    /// フィールド設定: PackSet(pack_var, field_index, value_var)
    PackSet(IrVar, usize, IrVar),
    /// フィールド型タグ設定: PackSetTag(pack_var, field_index, type_tag)
    /// type_tag: 0=Int, 1=Float, 2=Bool, 3=Str, 4=Pack, 5=List, 6=Closure
    PackSetTag(IrVar, usize, i64),
    /// フィールド取得: PackGet(dst, pack_var, field_index)
    PackGet(IrVar, IrVar, usize),

    // ── 関数呼び出し ──
    /// ランタイム関数呼び出し: `result = call(func_name, args...)`
    Call(IrVar, String, Vec<IrVar>),
    /// ユーザー定義関数呼び出し: `result = call_user(func_name, args...)`
    CallUser(IrVar, String, Vec<IrVar>),
    /// 間接関数呼び出し（ラムダ/クロージャ経由）: `result = call_indirect(fn_var, args...)`
    CallIndirect(IrVar, IrVar, Vec<IrVar>),

    // ── ラムダ/クロージャ ──
    /// クロージャ生成: MakeClosure(dst, func_name, captures)
    /// captures: キャプチャ変数名のリスト
    MakeClosure(IrVar, String, Vec<String>),
    /// 関数アドレス取得: FuncAddr(dst, func_name)
    FuncAddr(IrVar, String),

    // ── 制御フロー ──
    /// 条件分岐: CondBranch(result, arms)
    /// arms: Vec<(Option<IrVar>, Vec<IrInst>, IrVar)>
    ///   - condition (None = default case)
    ///   - body instructions
    ///   - result var of body
    CondBranch(IrVar, Vec<CondArm>),

    // ── 参照カウント（Phase N7） ──
    /// Retain: refcount++ (ヒープオブジェクトの参照カウントをインクリメント)
    Retain(IrVar),
    /// Release: refcount-- (0になったらfree)
    Release(IrVar),

    /// 末尾再帰呼び出し: 引数を再代入してエントリブロックにジャンプ
    /// TailCall(args) — 引数の IrVar リスト
    TailCall(Vec<IrVar>),

    // ── グローバル変数（トップレベル変数の関数間共有） ──
    /// グローバル変数への書き込み: GlobalSet(name_hash, value_var)
    GlobalSet(i64, IrVar),
    /// グローバル変数からの読み取り: GlobalGet(dst, name_hash)
    GlobalGet(IrVar, i64),

    Return(IrVar),
}

/// IR 関数定義
#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<IrInst>,
    pub next_var: IrVar,
}

impl IrFunction {
    pub fn new(name: String) -> Self {
        Self {
            name,
            params: Vec::new(),
            body: Vec::new(),
            next_var: 0,
        }
    }

    pub fn new_with_params(name: String, params: Vec<String>) -> Self {
        let next_var = params.len() as u32;
        Self {
            name,
            params,
            body: Vec::new(),
            next_var,
        }
    }

    pub fn alloc_var(&mut self) -> IrVar {
        let var = self.next_var;
        self.next_var += 1;
        var
    }

    pub fn push(&mut self, inst: IrInst) {
        self.body.push(inst);
    }
}

/// 条件分岐のアーム
#[derive(Debug, Clone)]
pub struct CondArm {
    /// 条件（None = デフォルトケース `| _ |>`）
    pub condition: Option<IrVar>,
    /// 本体の命令列
    pub body: Vec<IrInst>,
    /// 本体の結果変数
    pub result: IrVar,
}

/// IR モジュール（1ファイル = 1モジュール）
#[derive(Debug, Clone)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    /// エクスポートされるリンクシンボル名
    pub exports: Vec<String>,
    /// インポート: (モジュールパス, 依存する関数リンクシンボル名リスト)
    pub imports: Vec<(String, Vec<String>)>,
    /// ライブラリモジュール（エクスポートあり）かどうか
    pub is_library: bool,
    /// モジュール一意キー
    pub module_key: Option<String>,
}

impl Default for IrModule {
    fn default() -> Self {
        Self::new()
    }
}

impl IrModule {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            exports: Vec::new(),
            imports: Vec::new(),
            is_library: false,
            module_key: None,
        }
    }
}
