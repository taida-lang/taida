/// wasm-min C emitter -- Taida IR を C コードに変換し、clang で wasm32 object を生成
///
/// wasm-min は Cranelift の ISA に wasm32 が存在しないため、IR -> C -> clang -> wasm32 .o
/// というパイプラインを採用する。サポートする IR 命令は最小限:
///
/// - ConstInt, ConstFloat, ConstBool, ConstStr
/// - Call (runtime 関数のみ)
/// - CallUser (ユーザー定義関数)
/// - DefVar, UseVar
/// - CondBranch, Return, TailCall
/// - GlobalSet, GlobalGet
/// - Retain, Release (no-op)
///
/// クロージャ・BuchiPack は未対応。
/// 未対応 IR は silent miscompile ではなく compile error を返す。

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use super::ir::*;

#[derive(Debug)]
pub struct WasmCEmitError {
    pub message: String,
}

impl std::fmt::Display for WasmCEmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

// ---------------------------------------------------------------------------
// F-1: Capability validator -- 未対応 IR を compile error にする
// ---------------------------------------------------------------------------

/// wasm-min で未対応の IR 命令を検出して compile error にする。
/// silent miscompile を防ぐための事前バリデーション。
pub fn validate_wasm_min_capabilities(ir_module: &IrModule) -> Result<(), WasmCEmitError> {
    let mut unsupported = Vec::new();

    for func in &ir_module.functions {
        collect_unsupported_insts(&func.body, &func.name, &mut unsupported);
    }

    if unsupported.is_empty() {
        Ok(())
    } else {
        // Deduplicate by feature name
        let mut features: Vec<&str> = unsupported.iter().map(|s| s.as_str()).collect();
        features.sort();
        features.dedup();
        Err(WasmCEmitError {
            message: format!(
                "wasm-min does not support the following features: {}. \
                 Use the interpreter or native backend instead.",
                features.join(", ")
            ),
        })
    }
}

fn collect_unsupported_insts(insts: &[IrInst], _func_name: &str, out: &mut Vec<String>) {
    for inst in insts {
        match inst {
            // W-3: Float literals are now supported (f64 bits stored in int64_t via bitcast)
            IrInst::ConstFloat(_, _) => {}
            IrInst::PackNew(_, _) => {
                out.push("BuchiPack (PackNew)".to_string());
            }
            IrInst::PackGet(_, _, _) => {
                out.push("BuchiPack (PackGet)".to_string());
            }
            IrInst::PackSet(_, _, _) => {
                out.push("BuchiPack (PackSet)".to_string());
            }
            IrInst::PackSetTag(_, _, _) => {
                out.push("BuchiPack (PackSetTag)".to_string());
            }
            IrInst::FuncAddr(_, _) => {
                out.push("function references (FuncAddr)".to_string());
            }
            IrInst::MakeClosure(_, _, _) => {
                out.push("closures (MakeClosure)".to_string());
            }
            IrInst::CallIndirect(_, _, _) => {
                out.push("indirect calls (CallIndirect)".to_string());
            }
            IrInst::CondBranch(_, arms) => {
                for arm in arms {
                    collect_unsupported_insts(&arm.body, _func_name, out);
                }
            }
            // All supported instructions (ConstFloat is handled above)
            IrInst::ConstInt(_, _)
            | IrInst::ConstBool(_, _)
            | IrInst::ConstStr(_, _)
            | IrInst::DefVar(_, _)
            | IrInst::UseVar(_, _)
            | IrInst::Call(_, _, _)
            | IrInst::CallUser(_, _, _)
            | IrInst::Return(_)
            | IrInst::Retain(_)
            | IrInst::Release(_)
            | IrInst::GlobalSet(_, _)
            | IrInst::GlobalGet(_, _)
            | IrInst::TailCall(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// F-4: Global variable name collection (for name-based C variables)
// ---------------------------------------------------------------------------

/// Collect all global variable hashes used in the module and assign them
/// unique C variable names based on their hash values.
fn collect_global_hashes(ir_module: &IrModule) -> Vec<i64> {
    let mut hashes = HashSet::new();
    for func in &ir_module.functions {
        collect_global_hashes_from_insts(&func.body, &mut hashes);
    }
    let mut sorted: Vec<i64> = hashes.into_iter().collect();
    sorted.sort();
    sorted
}

fn collect_global_hashes_from_insts(insts: &[IrInst], hashes: &mut HashSet<i64>) {
    for inst in insts {
        match inst {
            IrInst::GlobalSet(hash, _) | IrInst::GlobalGet(_, hash) => {
                hashes.insert(*hash);
            }
            IrInst::CondBranch(_, arms) => {
                for arm in arms {
                    collect_global_hashes_from_insts(&arm.body, hashes);
                }
            }
            _ => {}
        }
    }
}

/// Build a map from hash -> C variable name for globals.
fn build_global_name_map(hashes: &[i64]) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    for (i, hash) in hashes.iter().enumerate() {
        // Use hash value in the name to avoid ambiguity, plus index for uniqueness
        let unsigned = *hash as u64;
        map.insert(*hash, format!("_tg_{}_{}", i, unsigned));
    }
    map
}

// ---------------------------------------------------------------------------
// C string literal helper
// ---------------------------------------------------------------------------

/// C コード上でのエスケープ済み文字列リテラルを生成
fn c_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_ascii_graphic() || c == ' ' => out.push(c),
            c => {
                // UTF-8 bytes as hex escapes
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                for b in encoded.bytes() {
                    write!(out, "\\x{:02x}", b).unwrap();
                }
            }
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Taida IR モジュールを wasm-min 用 C ソースに変換する
///
/// F-1: 事前に capability validation を実行し、未対応 IR は compile error にする。
pub fn emit_c(ir_module: &IrModule) -> Result<String, WasmCEmitError> {
    // F-1: capability validation (prevents silent miscompile)
    validate_wasm_min_capabilities(ir_module)?;

    let mut c = String::new();

    // ヘッダー
    writeln!(c, "/* Generated by Taida wasm-min C emitter */").unwrap();
    writeln!(c, "#include <stdint.h>").unwrap();
    writeln!(c).unwrap();

    // W-3: f64 -> i64 bitcast helper (union-based, no libc dependency)
    writeln!(c, "static int64_t _d2l(double v) {{ union {{ int64_t l; double d; }} u; u.d = v; return u.l; }}").unwrap();
    writeln!(c).unwrap();

    // F-4: グローバル変数を名前ベースの C 変数として宣言
    let global_hashes = collect_global_hashes(ir_module);
    let global_map = build_global_name_map(&global_hashes);
    for hash in &global_hashes {
        let var_name = &global_map[hash];
        writeln!(c, "static int64_t {};", var_name).unwrap();
    }
    if !global_hashes.is_empty() {
        writeln!(c).unwrap();
    }

    // runtime 関数のプロトタイプ宣言（必要なもののみ）
    let mut needed_funcs = HashSet::new();
    for func in &ir_module.functions {
        collect_needed_runtime_funcs(&func.body, &mut needed_funcs);
    }

    for name in &needed_funcs {
        writeln!(c, "{}", runtime_func_prototype(name)?).unwrap();
    }
    if !needed_funcs.is_empty() {
        writeln!(c).unwrap();
    }

    // ユーザー関数（forward declarations）
    for func in &ir_module.functions {
        if func.name != "_taida_main" {
            write!(c, "int64_t {}(", func.name).unwrap();
            for (i, _param) in func.params.iter().enumerate() {
                if i > 0 {
                    write!(c, ", ").unwrap();
                }
                write!(c, "int64_t").unwrap();
            }
            if func.params.is_empty() {
                write!(c, "void").unwrap();
            }
            writeln!(c, ");").unwrap();
        }
    }

    // 関数定義
    for func in &ir_module.functions {
        writeln!(c).unwrap();
        emit_function(&mut c, func, &global_map)?;
    }

    Ok(c)
}

fn collect_needed_runtime_funcs(insts: &[IrInst], set: &mut HashSet<String>) {
    for inst in insts {
        match inst {
            IrInst::Call(_, name, _) => {
                set.insert(name.clone());
            }
            IrInst::CondBranch(_, arms) => {
                for arm in arms {
                    collect_needed_runtime_funcs(&arm.body, set);
                }
            }
            _ => {}
        }
    }
}

/// runtime 関数の C プロトタイプを生成
///
/// wasm-min runtime では全値を int64_t (boxed value) として統一する。
/// 文字列ポインタも int64_t にキャストして渡す。runtime 側で適切にキャストする。
fn runtime_func_prototype(name: &str) -> Result<String, WasmCEmitError> {
    let proto = match name {
        // I/O
        "taida_io_stdout" | "taida_io_stderr" => {
            format!("int64_t {}(int64_t val);", name)
        }
        // Debug 出力 (W-3: taida_debug_float 追加)
        "taida_debug_int" | "taida_debug_str" | "taida_debug_bool"
        | "taida_debug_float" => {
            format!("int64_t {}(int64_t val);", name)
        }
        // 整数演算 (2引数)
        "taida_int_add" | "taida_int_sub" | "taida_int_mul" | "taida_int_eq"
        | "taida_int_neq" | "taida_int_lt" | "taida_int_gt" | "taida_int_gte" => {
            format!("int64_t {}(int64_t a, int64_t b);", name)
        }
        // 整数演算 (1引数)
        "taida_int_neg" => "int64_t taida_int_neg(int64_t a);".to_string(),
        // W-3: Float 演算 (boxed float as int64_t via bitcast)
        "taida_float_add" | "taida_float_sub" | "taida_float_mul" => {
            format!("int64_t {}(int64_t a, int64_t b);", name)
        }
        "taida_float_neg" => "int64_t taida_float_neg(int64_t a);".to_string(),
        // W-3: Int→Float 変換 (returns boxed float as int64_t)
        "taida_int_to_float" => "int64_t taida_int_to_float(int64_t a);".to_string(),
        // W-3: Float→Int 変換
        "taida_float_to_int" => "int64_t taida_float_to_int(int64_t a);".to_string(),
        // W-3: String operations
        "taida_str_concat" => "int64_t taida_str_concat(int64_t a, int64_t b);".to_string(),
        "taida_str_length" => "int64_t taida_str_length(int64_t s);".to_string(),
        "taida_str_eq" => "int64_t taida_str_eq(int64_t a, int64_t b);".to_string(),
        "taida_str_neq" => "int64_t taida_str_neq(int64_t a, int64_t b);".to_string(),
        // W-3: Type conversions
        "taida_int_to_str" => "int64_t taida_int_to_str(int64_t a);".to_string(),
        "taida_str_to_int" => "int64_t taida_str_to_int(int64_t s);".to_string(),
        "taida_str_from_bool" => "int64_t taida_str_from_bool(int64_t v);".to_string(),
        "taida_float_to_str" => "int64_t taida_float_to_str(int64_t a);".to_string(),
        "taida_int_abs" => "int64_t taida_int_abs(int64_t a);".to_string(),
        "taida_int_lte" => "int64_t taida_int_lte(int64_t a, int64_t b);".to_string(),
        // ブール演算
        "taida_bool_and" | "taida_bool_or" => {
            format!("int64_t {}(int64_t a, int64_t b);", name)
        }
        "taida_bool_not" => "int64_t taida_bool_not(int64_t a);".to_string(),
        // Div/Mod mold + unmold
        "taida_div_mold" | "taida_mod_mold" => {
            format!("int64_t {}(int64_t a, int64_t b);", name)
        }
        "taida_generic_unmold" => "int64_t taida_generic_unmold(int64_t val);".to_string(),
        // Polymorphic comparison
        "taida_poly_eq" | "taida_poly_neq" => {
            format!("int64_t {}(int64_t a, int64_t b);", name)
        }
        // RC no-ops
        "taida_retain" | "taida_release" => {
            format!("void {}(int64_t val);", name)
        }
        other => {
            // F-1: unsupported runtime functions are compile errors, not silent stubs
            return Err(WasmCEmitError {
                message: format!(
                    "wasm-min does not support runtime function '{}'. \
                     Use the interpreter or native backend instead.",
                    other
                ),
            });
        }
    };
    Ok(proto)
}

/// 現在の関数のパラメータ名（TailCall で使用）
struct FuncContext<'a> {
    param_names: Vec<String>,
    global_map: &'a HashMap<i64, String>,
}

/// 単一関数を C コードに変換
fn emit_function(
    c: &mut String,
    func: &IrFunction,
    global_map: &HashMap<i64, String>,
) -> Result<(), WasmCEmitError> {
    // 関数シグネチャ
    write!(c, "int64_t {}(", func.name).unwrap();
    for (i, param_name) in func.params.iter().enumerate() {
        if i > 0 {
            write!(c, ", ").unwrap();
        }
        write!(c, "int64_t v_{}", param_to_var_idx(param_name, i)).unwrap();
    }
    if func.params.is_empty() {
        write!(c, "void").unwrap();
    }
    writeln!(c, ") {{").unwrap();

    // ローカル変数の宣言（全 IrVar を事前宣言）
    // パラメータは既に関数引数として宣言されている
    let param_count = func.params.len() as u32;
    if func.next_var > param_count {
        for v in param_count..func.next_var {
            writeln!(c, "    int64_t v_{} = 0;", v).unwrap();
        }
    }

    // Named variables（DefVar/UseVar 用）
    let mut named_vars = HashSet::new();
    collect_named_vars(&func.body, &mut named_vars);
    // パラメータ名も named_vars に含める
    for param_name in &func.params {
        named_vars.insert(param_name.clone());
    }
    for name in &named_vars {
        writeln!(c, "    int64_t nv_{} = 0;", sanitize_name(name)).unwrap();
    }

    // パラメータを named variables にコピー（IR は DefVar なしで UseVar("n") を使う）
    for (i, param_name) in func.params.iter().enumerate() {
        writeln!(
            c,
            "    nv_{} = v_{};",
            sanitize_name(param_name),
            i
        )
        .unwrap();
    }

    let fctx = FuncContext {
        param_names: func.params.clone(),
        global_map,
    };

    // 末尾再帰のサポート: TailCall を含む場合はループで囲む
    let has_tail_call = contains_tail_call(&func.body);
    if has_tail_call {
        writeln!(c, "    while (1) {{").unwrap();
        emit_insts(c, &func.body, "        ", &fctx)?;
        writeln!(c, "    }}").unwrap();
    } else {
        // 命令列
        emit_insts(c, &func.body, "    ", &fctx)?;
    }

    // デフォルト return（最後の命令が Return でない場合）
    if !func.body.last().map_or(false, |i| matches!(i, IrInst::Return(_))) {
        writeln!(c, "    return 0;").unwrap();
    }

    writeln!(c, "}}").unwrap();
    Ok(())
}

fn collect_named_vars(insts: &[IrInst], set: &mut HashSet<String>) {
    for inst in insts {
        match inst {
            IrInst::DefVar(name, _) => {
                set.insert(name.clone());
            }
            IrInst::UseVar(_, name) => {
                set.insert(name.clone());
            }
            IrInst::CondBranch(_, arms) => {
                for arm in arms {
                    collect_named_vars(&arm.body, set);
                }
            }
            _ => {}
        }
    }
}

fn emit_insts(
    c: &mut String,
    insts: &[IrInst],
    indent: &str,
    fctx: &FuncContext,
) -> Result<(), WasmCEmitError> {
    for inst in insts {
        emit_inst(c, inst, indent, fctx)?;
    }
    Ok(())
}

fn emit_inst(
    c: &mut String,
    inst: &IrInst,
    indent: &str,
    fctx: &FuncContext,
) -> Result<(), WasmCEmitError> {
    match inst {
        IrInst::ConstInt(dst, val) => {
            writeln!(c, "{}v_{} = {}LL;", indent, dst, val).unwrap();
        }
        IrInst::ConstFloat(dst, val) => {
            // W-3: Store f64 bits in int64_t via bitcast (same representation as native backend)
            // Use _d2l() helper to bitcast double -> int64_t
            // Format with enough precision to round-trip
            writeln!(c, "{}v_{} = _d2l({:.17e});", indent, dst, val).unwrap();
        }
        IrInst::ConstBool(dst, val) => {
            writeln!(c, "{}v_{} = {};", indent, dst, if *val { 1 } else { 0 }).unwrap();
        }
        IrInst::ConstStr(dst, s) => {
            // 静的文字列リテラル: ポインタを i64 として格納
            writeln!(
                c,
                "{}v_{} = (int64_t)(intptr_t){};",
                indent,
                dst,
                c_string_literal(s)
            )
            .unwrap();
        }
        IrInst::DefVar(name, src) => {
            writeln!(c, "{}nv_{} = v_{};", indent, sanitize_name(name), src).unwrap();
        }
        IrInst::UseVar(dst, name) => {
            writeln!(c, "{}v_{} = nv_{};", indent, dst, sanitize_name(name)).unwrap();
        }
        IrInst::Call(dst, name, args) => {
            // RC no-ops: retain/release は void 返り値
            if name == "taida_retain" || name == "taida_release" {
                if let Some(arg) = args.first() {
                    writeln!(c, "{}{}(v_{});", indent, name, arg).unwrap();
                }
                writeln!(c, "{}v_{} = 0;", indent, dst).unwrap();
            } else {
                write!(c, "{}v_{} = {}(", indent, dst, name).unwrap();
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(c, ", ").unwrap();
                    }
                    write!(c, "v_{}", arg).unwrap();
                }
                writeln!(c, ");").unwrap();
            }
        }
        IrInst::CallUser(dst, name, args) => {
            write!(c, "{}v_{} = {}(", indent, dst, name).unwrap();
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    write!(c, ", ").unwrap();
                }
                write!(c, "v_{}", arg).unwrap();
            }
            writeln!(c, ");").unwrap();
        }
        IrInst::CondBranch(result, arms) => {
            for (i, arm) in arms.iter().enumerate() {
                if i == 0 {
                    if let Some(cond) = arm.condition {
                        writeln!(c, "{}if (v_{}) {{", indent, cond).unwrap();
                    } else {
                        writeln!(c, "{}{{", indent).unwrap();
                    }
                } else if arm.condition.is_some() {
                    writeln!(
                        c,
                        "{}}} else if (v_{}) {{",
                        indent,
                        arm.condition.unwrap()
                    )
                    .unwrap();
                } else {
                    writeln!(c, "{}}} else {{", indent).unwrap();
                }

                let inner_indent = format!("{}    ", indent);
                emit_insts(c, &arm.body, &inner_indent, fctx)?;
                writeln!(c, "{}    v_{} = v_{};", indent, result, arm.result).unwrap();
            }
            writeln!(c, "{}}}", indent).unwrap();
        }
        IrInst::Return(var) => {
            writeln!(c, "{}return v_{};", indent, var).unwrap();
        }
        // wasm-min で未サポートの命令
        IrInst::Retain(_) | IrInst::Release(_) => {
            // RC 操作は wasm-min では無視（ヒープなし）
            writeln!(c, "{}/* retain/release skipped (wasm-min) */", indent).unwrap();
        }
        // F-4: グローバル変数を名前ベースの C 変数で読み書き
        IrInst::GlobalSet(name_hash, value_var) => {
            let var_name = fctx.global_map.get(name_hash).expect("global hash not in map");
            writeln!(c, "{}{} = v_{};", indent, var_name, value_var).unwrap();
        }
        IrInst::GlobalGet(dst, name_hash) => {
            let var_name = fctx.global_map.get(name_hash).expect("global hash not in map");
            writeln!(c, "{}v_{} = {};", indent, dst, var_name).unwrap();
        }
        // F-1: These should never be reached because validate_wasm_min_capabilities
        // catches them before emission. But if somehow they slip through, error out.
        IrInst::PackNew(_, _)
        | IrInst::PackGet(_, _, _)
        | IrInst::PackSet(_, _, _)
        | IrInst::PackSetTag(_, _, _)
        | IrInst::FuncAddr(_, _)
        | IrInst::MakeClosure(_, _, _)
        | IrInst::CallIndirect(_, _, _) => {
            return Err(WasmCEmitError {
                message: "Internal error: unsupported IR instruction reached emit phase. \
                          This should have been caught by validate_wasm_min_capabilities."
                    .to_string(),
            });
        }
        IrInst::TailCall(args) => {
            // 末尾再帰: TailCall(args) の args を一時変数に評価してから
            // named variables を更新し、continue でループ先頭に戻る。
            // Cranelift emit.rs と同じく、全 args を先に評価してから代入する
            // （引数間の依存を回避するため）。
            for (i, arg) in args.iter().enumerate() {
                writeln!(c, "{}int64_t _tco_arg_{} = v_{};", indent, i, arg).unwrap();
            }
            for (i, param_name) in fctx.param_names.iter().enumerate() {
                if i < args.len() {
                    writeln!(
                        c,
                        "{}nv_{} = _tco_arg_{};",
                        indent,
                        sanitize_name(param_name),
                        i
                    )
                    .unwrap();
                }
            }
            writeln!(c, "{}continue;", indent).unwrap();
        }
    }
    Ok(())
}

/// IR 命令列に TailCall が含まれるかどうかを再帰的にチェック
fn contains_tail_call(insts: &[IrInst]) -> bool {
    for inst in insts {
        match inst {
            IrInst::TailCall(_) => return true,
            IrInst::CondBranch(_, arms) => {
                for arm in arms {
                    if contains_tail_call(&arm.body) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// パラメータ名からインデックスを生成（IrVar はパラメータ順に 0, 1, 2, ...）
fn param_to_var_idx(_name: &str, idx: usize) -> u32 {
    idx as u32
}

/// 変数名を C 識別子として安全な形に変換
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}
