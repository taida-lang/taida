/// コンパイルドライバ — .td → パース → IR → CLIF → .o → バイナリ
///
/// cranelift-object で .o を出力し、システムリンカでバイナリを生成。
/// wasm-min ターゲット時は wasm-ld で .wasm を生成する。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::emit::Emitter;
use super::emit_wasm_c;
use super::lower::Lowering;
use super::rc_opt;
use crate::module_graph;
use crate::parser::parse;

type ModuleImports = Vec<(String, Vec<String>)>;

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// 単一 .td ファイルを Native .o にコンパイル（リンクなし）
fn compile_to_object(input_path: &Path) -> Result<(PathBuf, ModuleImports), CompileError> {
    let source = fs::read_to_string(input_path).map_err(|e| CompileError {
        message: format!("failed to read '{}': {}", input_path.display(), e),
    })?;

    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        let msgs: Vec<String> = parse_errors.iter().map(|e| format!("{}", e)).collect();
        return Err(CompileError {
            message: format!("parse errors:\n{}", msgs.join("\n")),
        });
    }

    let mut lowering = Lowering::new();
    // QF-16/17: ソースディレクトリを設定（モジュールインポート解決用）
    if let Some(parent) = input_path.parent() {
        lowering.set_source_dir(parent.to_path_buf());
    }
    lowering.set_module_key(Lowering::module_key_for_path(input_path));
    let mut ir_module = lowering.lower_program(&program).map_err(|e| CompileError {
        message: format!("{}", e),
    })?;

    // RC 最適化パス: 不要な Retain/Release を除去
    rc_opt::optimize(&mut ir_module);

    let imports = ir_module.imports.clone();

    let mut emitter = Emitter::new().map_err(|e| CompileError {
        message: format!("{}", e),
    })?;
    emitter.emit_module(&ir_module).map_err(|e| CompileError {
        message: format!("{}", e),
    })?;

    let product = emitter.module.finish();
    let obj_bytes = product.emit().map_err(|e| CompileError {
        message: format!("object emission failed: {}", e),
    })?;

    let obj_path = input_path.with_extension("o");
    fs::write(&obj_path, &obj_bytes).map_err(|e| CompileError {
        message: format!("failed to write '{}': {}", obj_path.display(), e),
    })?;

    Ok((obj_path, imports))
}

/// .td ファイルをコンパイルしてネイティブバイナリを生成
pub fn compile_file(
    input_path: &Path,
    output_path: Option<&Path>,
) -> Result<PathBuf, CompileError> {
    module_graph::detect_local_import_cycle(input_path).map_err(|e| CompileError {
        message: e.to_string(),
    })?;

    let base_dir = input_path.parent().unwrap_or(Path::new("."));

    // メインファイルをコンパイル
    let (main_obj, imports) = compile_to_object(input_path)?;

    // 依存モジュールを再帰的にコンパイル
    let mut all_objs = vec![main_obj.clone()];
    let mut compiled = std::collections::HashSet::new();
    compiled.insert(
        input_path
            .canonicalize()
            .unwrap_or(input_path.to_path_buf()),
    );

    // Each pending import carries (module_path, symbols, importing_file_dir).
    // Relative paths are resolved from the importing file's directory, not the main file's.
    let mut pending_imports: Vec<(String, Vec<String>, PathBuf)> = imports
        .into_iter()
        .map(|(p, s)| (p, s, base_dir.to_path_buf()))
        .collect();
    while let Some((module_path, _symbols, importer_dir)) = pending_imports.pop() {
        let dep_path = resolve_module_path(&importer_dir, &module_path);
        let canonical = dep_path.canonicalize().unwrap_or(dep_path.clone());
        if compiled.contains(&canonical) {
            continue;
        }
        compiled.insert(canonical);

        let dep_dir = dep_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let (obj_path, sub_imports) = compile_to_object(&dep_path)?;
        all_objs.push(obj_path);
        pending_imports.extend(
            sub_imports
                .into_iter()
                .map(|(p, s)| (p, s, dep_dir.clone())),
        );
    }

    // リンカ呼び出し
    let bin_path = match output_path {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output");
            base_dir.join(stem)
        }
    };

    link_objects(&all_objs, &bin_path)?;

    // .o ファイルを削除
    for obj in &all_objs {
        let _ = fs::remove_file(obj);
    }

    Ok(bin_path)
}

/// モジュールパスの解決: "./math" → "./math.td"
fn resolve_module_path(base_dir: &Path, module_path: &str) -> PathBuf {
    let mut path = base_dir.join(module_path);
    if path.extension().is_none_or(|e| e != "td") {
        path.set_extension("td");
    }
    path
}

/// 複数 .o ファイルをリンクしてバイナリを生成
fn link_objects(obj_paths: &[PathBuf], bin_path: &Path) -> Result<(), CompileError> {
    // メイン .o のパスからエントリポイント C ファイルの場所を決定
    let obj_path = &obj_paths[0];
    link_objects_inner(obj_paths, obj_path, bin_path)
}

fn link_objects_inner(
    obj_paths: &[PathBuf],
    obj_path: &Path,
    bin_path: &Path,
) -> Result<(), CompileError> {
    // cc を使ってリンク（ランタイム関数はプロセス内に存在しないので、
    // スタブ .o も生成する必要がある）
    //
    // 戦略: main() エントリポイントを含む C ラッパーを生成し、
    // _taida_main を呼ぶ。ランタイム関数は Rust ライブラリとして
    // リンクする。
    //
    // Phase N1 では簡易アプローチ: C エントリポイント + ランタイムを
    // 別途コンパイルしてリンク。

    // C エントリポイントを生成
    let c_wrapper = include_str!("native_runtime.c");

    let c_path = obj_path.with_extension("_entry.c");
    fs::write(&c_path, c_wrapper).map_err(|e| CompileError {
        message: format!("failed to write C wrapper: {}", e),
    })?;

    // cc でコンパイル + リンク（-no-pie で PIE 警告を回避）
    let mut cmd = Command::new("cc");
    cmd.arg("-no-pie").arg(&c_path);
    for obj in obj_paths {
        cmd.arg(obj);
    }
    cmd.arg("-o").arg(bin_path).arg("-lm").arg("-lpthread");

    let status = cmd.status().map_err(|e| CompileError {
        message: format!("linker invocation failed: {}", e),
    })?;

    // 一時ファイルを削除
    let _ = fs::remove_file(&c_path);

    if !status.success() {
        return Err(CompileError {
            message: format!("linker failed with exit code: {:?}", status.code()),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// W-1: wasm-min コンパイルパス
// ---------------------------------------------------------------------------

/// wasm-ld の実行パスを検出する
fn find_wasm_ld() -> Result<PathBuf, CompileError> {
    // 1. PATH 上の wasm-ld
    if let Ok(output) = Command::new("which").arg("wasm-ld").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    // 2. 既知の LLVM インストール先を探索
    let candidates = [
        "/opt/rocm-6.4.2/lib/llvm/bin/wasm-ld",
        "/usr/lib/llvm-17/bin/wasm-ld",
        "/usr/lib/llvm-18/bin/wasm-ld",
        "/usr/lib/llvm-19/bin/wasm-ld",
        "/usr/lib/llvm-20/bin/wasm-ld",
        "/usr/local/bin/wasm-ld",
    ];
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Ok(p);
        }
    }

    Err(CompileError {
        message: "wasm-ld not found. Install LLVM/LLD (e.g. `apt install lld-17`).".to_string(),
    })
}

/// clang の実行パスを検出する（wasm32 クロスコンパイル用）
fn find_clang_for_wasm() -> Result<String, CompileError> {
    // バージョン付きの clang を優先的に検索
    for ver in &["17", "18", "19", "20"] {
        let name = format!("clang-{}", ver);
        if let Ok(output) = Command::new("which").arg(&name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
    }
    // フォールバック: PATH 上の clang
    if let Ok(output) = Command::new("which").arg("clang").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }

    Err(CompileError {
        message: "clang not found. Install clang (e.g. `apt install clang-17`).".to_string(),
    })
}

/// .td ファイルを wasm-min ターゲットでコンパイルし .wasm を生成する
///
/// wasm-min は単一ファイルのみ対応（モジュールインポート非対応）。
/// パイプライン: .td → parse → IR → C source → clang(wasm32) → .o → wasm-ld → .wasm
///
/// Cranelift の ISA に wasm32 が存在しないため、IR → C → clang ルートを採用する。
pub fn compile_file_wasm(
    input_path: &Path,
    output_path: Option<&Path>,
) -> Result<PathBuf, CompileError> {
    let source = fs::read_to_string(input_path).map_err(|e| CompileError {
        message: format!("failed to read '{}': {}", input_path.display(), e),
    })?;

    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        let msgs: Vec<String> = parse_errors.iter().map(|e| format!("{}", e)).collect();
        return Err(CompileError {
            message: format!("parse errors:\n{}", msgs.join("\n")),
        });
    }

    let mut lowering = Lowering::new();
    if let Some(parent) = input_path.parent() {
        lowering.set_source_dir(parent.to_path_buf());
    }
    lowering.set_module_key(Lowering::module_key_for_path(input_path));
    let mut ir_module = lowering.lower_program(&program).map_err(|e| CompileError {
        message: format!("{}", e),
    })?;

    // wasm-min ではモジュールインポートは未対応
    if !ir_module.imports.is_empty() {
        return Err(CompileError {
            message: "wasm-min does not support module imports.".to_string(),
        });
    }

    // RC 最適化パス（retain/release は C emitter で無視されるが、IR を整える）
    rc_opt::optimize(&mut ir_module);

    // IR → C ソースコード
    let generated_c = emit_wasm_c::emit_c(&ir_module).map_err(|e| CompileError {
        message: format!("wasm-min C emission failed: {}", e),
    })?;

    let base_dir = input_path.parent().unwrap_or(Path::new("."));

    // 出力パスの決定
    let wasm_path = match output_path {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output");
            base_dir.join(format!("{}.wasm", stem))
        }
    };

    // 一時ファイルパスの生成（出力パスベースで一意化し、並列コンパイルの衝突を防ぐ）
    let tmp_base = wasm_path.with_extension("_wasm_tmp");
    let gen_c_path = tmp_base.with_extension("gen.c");
    let gen_obj_path = tmp_base.with_extension("gen.o");
    let rt_c_path = tmp_base.with_extension("rt.c");
    let rt_obj_path = tmp_base.with_extension("rt.o");

    // 生成された C ソースを書き出し
    fs::write(&gen_c_path, &generated_c).map_err(|e| CompileError {
        message: format!("failed to write generated C: {}", e),
    })?;

    // runtime_core_wasm.c を書き出し
    let rt_source = include_str!("runtime_core_wasm.c");
    fs::write(&rt_c_path, rt_source).map_err(|e| CompileError {
        message: format!("failed to write wasm runtime source: {}", e),
    })?;

    let clang = find_clang_for_wasm()?;

    // 生成 C をコンパイル
    let gen_status = Command::new(&clang)
        .args([
            "--target=wasm32-unknown-wasi",
            "-nostdlib",
            "-O2",
            "-c",
        ])
        .arg(&gen_c_path)
        .arg("-o")
        .arg(&gen_obj_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("clang invocation failed: {}", e),
        })?;

    if !gen_status.success() {
        // C ソースをデバッグ用に残す
        let _ = fs::remove_file(&rt_c_path);
        let _ = fs::remove_file(&rt_obj_path);
        return Err(CompileError {
            message: format!(
                "clang wasm32 compilation of generated code failed (source preserved at: {})",
                gen_c_path.display()
            ),
        });
    }

    // runtime をコンパイル
    let rt_status = Command::new(&clang)
        .args([
            "--target=wasm32-unknown-wasi",
            "-nostdlib",
            "-O2",
            "-c",
        ])
        .arg(&rt_c_path)
        .arg("-o")
        .arg(&rt_obj_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("clang invocation failed: {}", e),
        })?;

    let _ = fs::remove_file(&gen_c_path);
    let _ = fs::remove_file(&rt_c_path);

    if !rt_status.success() {
        let _ = fs::remove_file(&gen_obj_path);
        return Err(CompileError {
            message: "clang wasm32 compilation of runtime failed.".to_string(),
        });
    }

    // wasm-ld でリンク
    let wasm_ld = find_wasm_ld()?;
    let ld_status = Command::new(&wasm_ld)
        .args([
            "--no-entry",
            "--export=_start",
            "--strip-all",
            "--gc-sections",
        ])
        .arg(&rt_obj_path)
        .arg(&gen_obj_path)
        .arg("-o")
        .arg(&wasm_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("wasm-ld invocation failed: {}", e),
        })?;

    // 一時ファイルの削除
    let _ = fs::remove_file(&gen_obj_path);
    let _ = fs::remove_file(&rt_obj_path);

    if !ld_status.success() {
        return Err(CompileError {
            message: format!(
                "wasm-ld failed with exit code: {:?}",
                ld_status.code()
            ),
        });
    }

    Ok(wasm_path)
}
