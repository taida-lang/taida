/// コンパイルドライバ — .td → パース → IR → CLIF → .o → バイナリ
///
/// cranelift-object で .o を出力し、システムリンカでバイナリを生成。
/// wasm-min ターゲット時は wasm-ld で .wasm を生成する。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

use super::emit::Emitter;
use super::emit_wasm_c;
use super::lower::Lowering;
use super::rc_opt;
use crate::module_graph;
use crate::parser::parse;

type ModuleImports = Vec<(String, Vec<String>)>;

/// Process-wide counter to produce unique .o file names.
static OBJ_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Return a unique .o path derived from `input_path` by appending PID and a
/// monotonic counter.  This prevents races when two threads (or two processes)
/// compile the same .td file concurrently.
fn unique_obj_path(input_path: &Path) -> PathBuf {
    let pid = std::process::id();
    let seq = OBJ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("taida_obj");
    let dir = input_path.parent().unwrap_or(Path::new("."));
    dir.join(format!("{}.{}.{}.o", stem, pid, seq))
}

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Output of wasm-edge compilation: .wasm binary + JS glue for Workers deployment.
#[derive(Debug)]
pub struct WasmEdgeOutput {
    pub wasm_path: PathBuf,
    pub glue_path: PathBuf,
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

    let obj_path = unique_obj_path(input_path);
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

    let result = (|| -> Result<PathBuf, CompileError> {
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
        Ok(bin_path)
    })();

    // .o ファイルを削除（エラー時も確実にクリーンアップ）
    for obj in &all_objs {
        let _ = fs::remove_file(obj);
    }

    result
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
    if let Ok(output) = Command::new("which").arg("wasm-ld").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
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
        if let Ok(output) = Command::new("which").arg(&name).output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    // フォールバック: PATH 上の clang
    if let Ok(output) = Command::new("which").arg("clang").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    Err(CompileError {
        message: "clang not found. Install clang (e.g. `apt install clang-17`).".to_string(),
    })
}

const WASM_STDINT_HEADER: &str = r#"#ifndef TAIDA_WASM_STDINT_H
#define TAIDA_WASM_STDINT_H

typedef signed char int8_t;
typedef unsigned char uint8_t;
typedef short int16_t;
typedef unsigned short uint16_t;
typedef int int32_t;
typedef unsigned int uint32_t;
typedef long long int64_t;
typedef unsigned long long uint64_t;
typedef int intptr_t;
typedef unsigned int uintptr_t;

#endif
"#;

fn write_wasm_stdint_header(include_dir: &Path) -> Result<(), CompileError> {
    fs::create_dir_all(include_dir).map_err(|e| CompileError {
        message: format!(
            "failed to create wasm include dir '{}': {}",
            include_dir.display(),
            e
        ),
    })?;

    if let Err(e) = fs::write(include_dir.join("stdint.h"), WASM_STDINT_HEADER) {
        let _ = fs::remove_dir_all(include_dir);
        return Err(CompileError {
            message: format!(
                "failed to write wasm stdint shim into '{}': {}",
                include_dir.display(),
                e
            ),
        });
    }

    Ok(())
}

fn wasm_clang_base_args(include_dir: &Path) -> Vec<String> {
    vec![
        "--target=wasm32-unknown-wasi".to_string(),
        "-nostdlib".to_string(),
        "-O2".to_string(),
        "-c".to_string(),
        "-I".to_string(),
        include_dir.display().to_string(),
    ]
}

#[allow(clippy::too_many_arguments)]
fn run_wasm_clang_object(
    clang: &str,
    clang_args: &[String],
    input: &Path,
    output: &Path,
    include_dir: &Path,
    cleanup_paths: &[&Path],
    cleanup_include_dir_on_error: bool,
    preserved_source: Option<&Path>,
) -> Result<ExitStatus, CompileError> {
    Command::new(clang)
        .args(clang_args)
        .arg(input)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|e| {
            for path in cleanup_paths {
                let _ = fs::remove_file(path);
            }
            if cleanup_include_dir_on_error {
                let _ = fs::remove_dir_all(include_dir);
            }
            CompileError {
                message: match preserved_source {
                    Some(source) => format!(
                        "clang invocation failed: {} (source preserved at: {}; shim preserved at: {})",
                        e,
                        source.display(),
                        include_dir.display()
                    ),
                    None => format!("clang invocation failed: {}", e),
                },
            }
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
    let generated_c =
        emit_wasm_c::emit_c(&ir_module, emit_wasm_c::WasmProfile::Min).map_err(|e| {
            CompileError {
                message: format!("wasm-min C emission failed: {}", e),
            }
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
    let include_dir = tmp_base.with_extension("include");

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
    if let Err(err) = write_wasm_stdint_header(&include_dir) {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&rt_c_path);
        return Err(err);
    }

    // 生成 C をコンパイル
    let clang_args = wasm_clang_base_args(&include_dir);

    let gen_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &gen_c_path,
        &gen_obj_path,
        &include_dir,
        &[],
        false,
        Some(&gen_c_path),
    )?;

    if !gen_status.success() {
        // C ソースをデバッグ用に残す
        let _ = fs::remove_file(&rt_c_path);
        let _ = fs::remove_file(&rt_obj_path);
        return Err(CompileError {
            message: format!(
                "clang wasm32 compilation of generated code failed (source preserved at: {}; shim preserved at: {})",
                gen_c_path.display(),
                include_dir.display()
            ),
        });
    }

    // runtime をコンパイル
    let rt_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_c_path,
        &rt_obj_path,
        &include_dir,
        &[&gen_c_path, &rt_c_path, &gen_obj_path],
        true,
        None,
    )?;

    let _ = fs::remove_file(&gen_c_path);
    let _ = fs::remove_file(&rt_c_path);
    let _ = fs::remove_dir_all(&include_dir);

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
            message: format!("wasm-ld failed with exit code: {:?}", ld_status.code()),
        });
    }

    Ok(wasm_path)
}

// ---------------------------------------------------------------------------
// WW-2: wasm-wasi コンパイルパス
// ---------------------------------------------------------------------------

/// .td ファイルを wasm-wasi ターゲットでコンパイルし .wasm を生成する
///
/// wasm-wasi は wasm-min の上位互換で、WASI I/O (env, file read/write) を追加する。
/// パイプライン: .td → parse → IR → C source → clang(wasm32) → .o → wasm-ld → .wasm
///
/// リンク構成: gen.o + rt_core.o + rt_wasi.o → wasm-ld → output.wasm
pub fn compile_file_wasm_wasi(
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

    // wasm-wasi でもモジュールインポートは未対応（wasm-min と同じ制約）
    if !ir_module.imports.is_empty() {
        return Err(CompileError {
            message: "wasm-wasi does not support module imports.".to_string(),
        });
    }

    // RC 最適化パス
    rc_opt::optimize(&mut ir_module);

    // IR → C ソースコード (wasm-wasi profile: OS API prototypes allowed)
    let generated_c =
        emit_wasm_c::emit_c(&ir_module, emit_wasm_c::WasmProfile::Wasi).map_err(|e| {
            CompileError {
                message: format!("wasm-wasi C emission failed: {}", e),
            }
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

    // 一時ファイルパスの生成
    let tmp_base = wasm_path.with_extension("_wasm_wasi_tmp");
    let gen_c_path = tmp_base.with_extension("gen.c");
    let gen_obj_path = tmp_base.with_extension("gen.o");
    let rt_core_c_path = tmp_base.with_extension("rt_core.c");
    let rt_core_obj_path = tmp_base.with_extension("rt_core.o");
    let rt_wasi_c_path = tmp_base.with_extension("rt_wasi.c");
    let rt_wasi_obj_path = tmp_base.with_extension("rt_wasi.o");
    let include_dir = tmp_base.with_extension("include");

    // 生成された C ソースを書き出し
    fs::write(&gen_c_path, &generated_c).map_err(|e| CompileError {
        message: format!("failed to write generated C: {}", e),
    })?;

    // runtime_core_wasm.c を書き出し（凍結、wasm-min と同一）
    let rt_core_source = include_str!("runtime_core_wasm.c");
    fs::write(&rt_core_c_path, rt_core_source).map_err(|e| CompileError {
        message: format!("failed to write wasm core runtime source: {}", e),
    })?;

    // runtime_wasi_io.c を書き出し（wasm-wasi 専用 I/O 層）
    let rt_wasi_source = include_str!("runtime_wasi_io.c");
    fs::write(&rt_wasi_c_path, rt_wasi_source).map_err(|e| CompileError {
        message: format!("failed to write wasm WASI I/O source: {}", e),
    })?;
    let clang = find_clang_for_wasm()?;
    if let Err(err) = write_wasm_stdint_header(&include_dir) {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        return Err(err);
    }

    // 生成 C をコンパイル
    let clang_args = wasm_clang_base_args(&include_dir);

    let gen_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &gen_c_path,
        &gen_obj_path,
        &include_dir,
        &[],
        false,
        Some(&gen_c_path),
    )?;

    if !gen_status.success() {
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        return Err(CompileError {
            message: format!(
                "clang wasm32 compilation of generated code failed (source preserved at: {}; shim preserved at: {})",
                gen_c_path.display(),
                include_dir.display()
            ),
        });
    }

    // runtime core をコンパイル
    let rt_core_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_core_c_path,
        &rt_core_obj_path,
        &include_dir,
        &[&gen_c_path, &gen_obj_path, &rt_core_c_path, &rt_wasi_c_path],
        true,
        None,
    )?;

    if !rt_core_status.success() {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        let _ = fs::remove_dir_all(&include_dir);
        return Err(CompileError {
            message: "clang wasm32 compilation of core runtime failed.".to_string(),
        });
    }

    // runtime WASI I/O をコンパイル
    let rt_wasi_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_wasi_c_path,
        &rt_wasi_obj_path,
        &include_dir,
        &[
            &gen_c_path,
            &gen_obj_path,
            &rt_core_c_path,
            &rt_core_obj_path,
            &rt_wasi_c_path,
        ],
        true,
        None,
    )?;

    // 一時 C ソースを削除
    let _ = fs::remove_file(&gen_c_path);
    let _ = fs::remove_file(&rt_core_c_path);
    let _ = fs::remove_file(&rt_wasi_c_path);
    let _ = fs::remove_dir_all(&include_dir);

    if !rt_wasi_status.success() {
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_obj_path);
        return Err(CompileError {
            message: "clang wasm32 compilation of WASI I/O runtime failed.".to_string(),
        });
    }

    // wasm-ld でリンク（3 object: gen.o + rt_core.o + rt_wasi.o）
    let wasm_ld = find_wasm_ld()?;
    let ld_status = Command::new(&wasm_ld)
        .args([
            "--no-entry",
            "--export=_start",
            "--strip-all",
            "--gc-sections",
        ])
        .arg(&rt_core_obj_path)
        .arg(&rt_wasi_obj_path)
        .arg(&gen_obj_path)
        .arg("-o")
        .arg(&wasm_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("wasm-ld invocation failed: {}", e),
        })?;

    // 一時ファイルの削除
    let _ = fs::remove_file(&gen_obj_path);
    let _ = fs::remove_file(&rt_core_obj_path);
    let _ = fs::remove_file(&rt_wasi_obj_path);

    if !ld_status.success() {
        return Err(CompileError {
            message: format!("wasm-ld failed with exit code: {:?}", ld_status.code()),
        });
    }

    Ok(wasm_path)
}

/// wasm-edge: Cloudflare Workers 向け edge profile
///
/// wasm-min の上に host import (taida_host) ベースの env API を追加する。
/// WASI import はそのまま残し、JS glue が wasi_snapshot_preview1.fd_write を提供する。
///
/// パイプライン: .td -> parse -> IR -> C source -> clang(wasm32) -> .o -> wasm-ld -> .wasm
///
/// リンク構成: gen.o + rt_core.o + rt_edge.o -> wasm-ld -> output.wasm
pub fn compile_file_wasm_edge(
    input_path: &Path,
    output_path: Option<&Path>,
) -> Result<WasmEdgeOutput, CompileError> {
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

    // wasm-edge でもモジュールインポートは未対応
    if !ir_module.imports.is_empty() {
        return Err(CompileError {
            message: "wasm-edge does not support module imports.".to_string(),
        });
    }

    // RC 最適化パス
    rc_opt::optimize(&mut ir_module);

    // IR -> C ソースコード (wasm-edge profile: env API allowed, file I/O rejected)
    let generated_c =
        emit_wasm_c::emit_c(&ir_module, emit_wasm_c::WasmProfile::Edge).map_err(|e| {
            CompileError {
                message: format!("wasm-edge C emission failed: {}", e),
            }
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

    // 一時ファイルパスの生成
    let tmp_base = wasm_path.with_extension("_wasm_edge_tmp");
    let gen_c_path = tmp_base.with_extension("gen.c");
    let gen_obj_path = tmp_base.with_extension("gen.o");
    let rt_core_c_path = tmp_base.with_extension("rt_core.c");
    let rt_core_obj_path = tmp_base.with_extension("rt_core.o");
    let rt_edge_c_path = tmp_base.with_extension("rt_edge.c");
    let rt_edge_obj_path = tmp_base.with_extension("rt_edge.o");
    let include_dir = tmp_base.with_extension("include");

    // 生成された C ソースを書き出し
    fs::write(&gen_c_path, &generated_c).map_err(|e| CompileError {
        message: format!("failed to write generated C: {}", e),
    })?;

    // runtime_core_wasm.c を書き出し（凍結、全 profile 共通）
    let rt_core_source = include_str!("runtime_core_wasm.c");
    fs::write(&rt_core_c_path, rt_core_source).map_err(|e| CompileError {
        message: format!("failed to write wasm core runtime source: {}", e),
    })?;

    // runtime_edge_host.c を書き出し（wasm-edge 専用 host import 層）
    let rt_edge_source = include_str!("runtime_edge_host.c");
    fs::write(&rt_edge_c_path, rt_edge_source).map_err(|e| CompileError {
        message: format!("failed to write wasm edge host source: {}", e),
    })?;
    let clang = find_clang_for_wasm()?;
    if let Err(err) = write_wasm_stdint_header(&include_dir) {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_edge_c_path);
        return Err(err);
    }

    // 生成 C をコンパイル
    let clang_args = wasm_clang_base_args(&include_dir);

    let gen_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &gen_c_path,
        &gen_obj_path,
        &include_dir,
        &[],
        false,
        Some(&gen_c_path),
    )?;

    if !gen_status.success() {
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_edge_c_path);
        return Err(CompileError {
            message: format!(
                "clang wasm32 compilation of generated code failed (source preserved at: {}; shim preserved at: {})",
                gen_c_path.display(),
                include_dir.display()
            ),
        });
    }

    // runtime core をコンパイル
    let rt_core_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_core_c_path,
        &rt_core_obj_path,
        &include_dir,
        &[&gen_c_path, &gen_obj_path, &rt_core_c_path, &rt_edge_c_path],
        true,
        None,
    )?;

    if !rt_core_status.success() {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_edge_c_path);
        let _ = fs::remove_dir_all(&include_dir);
        return Err(CompileError {
            message: "clang wasm32 compilation of core runtime failed.".to_string(),
        });
    }

    // runtime edge host をコンパイル
    let rt_edge_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_edge_c_path,
        &rt_edge_obj_path,
        &include_dir,
        &[
            &gen_c_path,
            &gen_obj_path,
            &rt_core_c_path,
            &rt_core_obj_path,
            &rt_edge_c_path,
        ],
        true,
        None,
    )?;

    // 一時 C ソースを削除
    let _ = fs::remove_file(&gen_c_path);
    let _ = fs::remove_file(&rt_core_c_path);
    let _ = fs::remove_file(&rt_edge_c_path);
    let _ = fs::remove_dir_all(&include_dir);

    if !rt_edge_status.success() {
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_obj_path);
        return Err(CompileError {
            message: "clang wasm32 compilation of edge host runtime failed.".to_string(),
        });
    }

    // wasm-ld でリンク（3 object: gen.o + rt_core.o + rt_edge.o）
    let wasm_ld = find_wasm_ld()?;
    let ld_status = Command::new(&wasm_ld)
        .args([
            "--no-entry",
            "--export=_start",
            "--strip-all",
            "--gc-sections",
        ])
        .arg(&rt_core_obj_path)
        .arg(&rt_edge_obj_path)
        .arg(&gen_obj_path)
        .arg("-o")
        .arg(&wasm_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("wasm-ld invocation failed: {}", e),
        })?;

    // 一時ファイルの削除
    let _ = fs::remove_file(&gen_obj_path);
    let _ = fs::remove_file(&rt_core_obj_path);
    let _ = fs::remove_file(&rt_edge_obj_path);

    if !ld_status.success() {
        return Err(CompileError {
            message: format!("wasm-ld failed with exit code: {:?}", ld_status.code()),
        });
    }

    // WE-2d: Generate JS glue alongside the .wasm
    let glue_path = generate_edge_js_glue(&wasm_path)?;

    Ok(WasmEdgeOutput {
        wasm_path,
        glue_path,
    })
}

// ---------------------------------------------------------------------------
// WF-2: wasm-full -- extended runtime profile (superset of wasm-wasi)
// ---------------------------------------------------------------------------

/// wasm-full: 拡張 runtime profile
///
/// wasm-wasi の上位互換として、文字列/数値 molds, 拡張 list/hashmap/set,
/// JSON, bytes, bitwise 等の重い runtime を追加する。
///
/// パイプライン: .td -> parse -> IR -> C source -> clang(wasm32) -> .o -> wasm-ld -> .wasm
///
/// リンク構成: gen.o + rt_core.o + rt_wasi.o + rt_full.o -> wasm-ld -> output.wasm
pub fn compile_file_wasm_full(
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

    // wasm-full でもモジュールインポートは未対応
    if !ir_module.imports.is_empty() {
        return Err(CompileError {
            message: "wasm-full does not support module imports.".to_string(),
        });
    }

    // RC 最適化パス
    rc_opt::optimize(&mut ir_module);

    // IR -> C ソースコード (wasm-full profile)
    let generated_c =
        emit_wasm_c::emit_c(&ir_module, emit_wasm_c::WasmProfile::Full).map_err(|e| {
            CompileError {
                message: format!("wasm-full C emission failed: {}", e),
            }
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

    // 一時ファイルパスの生成
    let tmp_base = wasm_path.with_extension("_wasm_full_tmp");
    let gen_c_path = tmp_base.with_extension("gen.c");
    let gen_obj_path = tmp_base.with_extension("gen.o");
    let rt_core_c_path = tmp_base.with_extension("rt_core.c");
    let rt_core_obj_path = tmp_base.with_extension("rt_core.o");
    let rt_wasi_c_path = tmp_base.with_extension("rt_wasi.c");
    let rt_wasi_obj_path = tmp_base.with_extension("rt_wasi.o");
    let rt_full_c_path = tmp_base.with_extension("rt_full.c");
    let rt_full_obj_path = tmp_base.with_extension("rt_full.o");
    let include_dir = tmp_base.with_extension("include");

    // 生成された C ソースを書き出し
    fs::write(&gen_c_path, &generated_c).map_err(|e| CompileError {
        message: format!("failed to write generated C: {}", e),
    })?;

    // runtime_core_wasm.c (凍結)
    let rt_core_source = include_str!("runtime_core_wasm.c");
    fs::write(&rt_core_c_path, rt_core_source).map_err(|e| CompileError {
        message: format!("failed to write wasm core runtime source: {}", e),
    })?;

    // runtime_wasi_io.c (wasm-wasi I/O 層)
    let rt_wasi_source = include_str!("runtime_wasi_io.c");
    fs::write(&rt_wasi_c_path, rt_wasi_source).map_err(|e| CompileError {
        message: format!("failed to write wasm WASI I/O source: {}", e),
    })?;

    // runtime_full_wasm.c (wasm-full 拡張層)
    let rt_full_source = include_str!("runtime_full_wasm.c");
    fs::write(&rt_full_c_path, rt_full_source).map_err(|e| CompileError {
        message: format!("failed to write wasm full runtime source: {}", e),
    })?;
    let clang = find_clang_for_wasm()?;
    if let Err(err) = write_wasm_stdint_header(&include_dir) {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        let _ = fs::remove_file(&rt_full_c_path);
        return Err(err);
    }

    // 生成 C をコンパイル
    let clang_args = wasm_clang_base_args(&include_dir);

    let gen_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &gen_c_path,
        &gen_obj_path,
        &include_dir,
        &[],
        false,
        Some(&gen_c_path),
    )?;

    if !gen_status.success() {
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        let _ = fs::remove_file(&rt_full_c_path);
        return Err(CompileError {
            message: format!(
                "clang wasm32 compilation of generated code failed (source preserved at: {}; shim preserved at: {})",
                gen_c_path.display(),
                include_dir.display()
            ),
        });
    }

    // runtime core をコンパイル
    let rt_core_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_core_c_path,
        &rt_core_obj_path,
        &include_dir,
        &[
            &gen_c_path,
            &gen_obj_path,
            &rt_core_c_path,
            &rt_wasi_c_path,
            &rt_full_c_path,
        ],
        true,
        None,
    )?;

    if !rt_core_status.success() {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        let _ = fs::remove_file(&rt_full_c_path);
        let _ = fs::remove_dir_all(&include_dir);
        return Err(CompileError {
            message: "clang wasm32 compilation of core runtime failed.".to_string(),
        });
    }

    // runtime WASI I/O をコンパイル
    let rt_wasi_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_wasi_c_path,
        &rt_wasi_obj_path,
        &include_dir,
        &[
            &gen_c_path,
            &gen_obj_path,
            &rt_core_c_path,
            &rt_core_obj_path,
            &rt_wasi_c_path,
            &rt_full_c_path,
        ],
        true,
        None,
    )?;

    if !rt_wasi_status.success() {
        let _ = fs::remove_file(&gen_c_path);
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_obj_path);
        let _ = fs::remove_file(&rt_core_c_path);
        let _ = fs::remove_file(&rt_wasi_c_path);
        let _ = fs::remove_file(&rt_full_c_path);
        let _ = fs::remove_dir_all(&include_dir);
        return Err(CompileError {
            message: "clang wasm32 compilation of WASI I/O runtime failed.".to_string(),
        });
    }

    // runtime full をコンパイル
    let rt_full_status = run_wasm_clang_object(
        &clang,
        &clang_args,
        &rt_full_c_path,
        &rt_full_obj_path,
        &include_dir,
        &[
            &gen_c_path,
            &gen_obj_path,
            &rt_core_c_path,
            &rt_core_obj_path,
            &rt_wasi_c_path,
            &rt_wasi_obj_path,
            &rt_full_c_path,
        ],
        true,
        None,
    )?;

    // 一時 C ソースを削除
    let _ = fs::remove_file(&gen_c_path);
    let _ = fs::remove_file(&rt_core_c_path);
    let _ = fs::remove_file(&rt_wasi_c_path);
    let _ = fs::remove_file(&rt_full_c_path);
    let _ = fs::remove_dir_all(&include_dir);

    if !rt_full_status.success() {
        let _ = fs::remove_file(&gen_obj_path);
        let _ = fs::remove_file(&rt_core_obj_path);
        let _ = fs::remove_file(&rt_wasi_obj_path);
        return Err(CompileError {
            message: "clang wasm32 compilation of full runtime failed.".to_string(),
        });
    }

    // wasm-ld でリンク（4 object: gen.o + rt_core.o + rt_wasi.o + rt_full.o）
    let wasm_ld = find_wasm_ld()?;
    let ld_status = Command::new(&wasm_ld)
        .args([
            "--no-entry",
            "--export=_start",
            "--strip-all",
            "--gc-sections",
        ])
        .arg(&rt_core_obj_path)
        .arg(&rt_wasi_obj_path)
        .arg(&rt_full_obj_path)
        .arg(&gen_obj_path)
        .arg("-o")
        .arg(&wasm_path)
        .status()
        .map_err(|e| CompileError {
            message: format!("wasm-ld invocation failed: {}", e),
        })?;

    // 一時ファイルの削除
    let _ = fs::remove_file(&gen_obj_path);
    let _ = fs::remove_file(&rt_core_obj_path);
    let _ = fs::remove_file(&rt_wasi_obj_path);
    let _ = fs::remove_file(&rt_full_obj_path);

    if !ld_status.success() {
        return Err(CompileError {
            message: format!("wasm-ld failed with exit code: {:?}", ld_status.code()),
        });
    }

    Ok(wasm_path)
}

// ---------------------------------------------------------------------------
// WE-2d: JS glue generation for Cloudflare Workers
// ---------------------------------------------------------------------------

/// Generate a JS glue file for Cloudflare Workers deployment.
///
/// The glue provides:
/// - `wasi_snapshot_preview1.fd_write` bridged to Response body capture
/// - `taida_host.env_get` / `taida_host.env_get_all` bridged to Workers `env`
/// - Workers `export default { fetch }` entrypoint
///
/// Output: `{stem}.edge.js` next to the `.wasm` file.
fn generate_edge_js_glue(wasm_path: &Path) -> Result<PathBuf, CompileError> {
    let stem = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let wasm_filename = wasm_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output.wasm");

    let glue_path = wasm_path.with_extension("edge.js");

    let glue = generate_edge_js_source(stem, wasm_filename);

    fs::write(&glue_path, glue).map_err(|e| CompileError {
        message: format!("failed to write JS glue '{}': {}", glue_path.display(), e),
    })?;

    Ok(glue_path)
}

/// Generate the JS glue source code for Cloudflare Workers.
///
/// This is a pure function (no I/O) to facilitate testing.
pub fn generate_edge_js_source(stem: &str, wasm_filename: &str) -> String {
    format!(
        r#"// {stem}.edge.js -- Cloudflare Workers glue for Taida wasm-edge
// Generated by `taida build --target wasm-edge`
//
// Deploy: wrangler deploy --name {stem}
// wrangler.toml should set:
//   main = "{stem}.edge.js"
//   [wasm_modules]
//   WASM = "{wasm_filename}"

import WASM from "./{wasm_filename}";

export default {{
  async fetch(request, env, ctx) {{
    const stdoutChunks = [];
    const stderrChunks = [];

    let memory = new WebAssembly.Memory({{ initial: 2 }});
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();

    // -- helpers --

    function readStr(ptr, len) {{
      return decoder.decode(new Uint8Array(memory.buffer, ptr, len));
    }}

    function writeStr(ptr, str) {{
      const bytes = encoder.encode(str);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      return bytes.length;
    }}

    // -- wasi_snapshot_preview1 --

    const wasi = {{
      fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) {{
        const view = new DataView(memory.buffer);
        let total = 0;
        for (let i = 0; i < iovs_len; i++) {{
          const base = iovs_ptr + i * 8;
          const ptr = view.getUint32(base, true);
          const len = view.getUint32(base + 4, true);
          const chunk = new Uint8Array(memory.buffer, ptr, len);
          if (fd === 1) {{
            stdoutChunks.push(new Uint8Array(chunk));
          }} else if (fd === 2) {{
            stderrChunks.push(new Uint8Array(chunk));
          }}
          total += len;
        }}
        view.setUint32(nwritten_ptr, total, true);
        return 0;
      }},
    }};

    // -- taida_host --

    const taida_host = {{
      env_get(key_ptr, key_len, buf_ptr, buf_cap) {{
        const key = readStr(key_ptr, key_len);
        const val = env[key];
        if (val === undefined || val === null || typeof val !== "string") {{
          return 0;
        }}
        const bytes = encoder.encode(val);
        if (bytes.length > buf_cap) {{
          return bytes.length;
        }}
        new Uint8Array(memory.buffer, buf_ptr, bytes.length).set(bytes);
        return bytes.length;
      }},

      env_get_all(buf_ptr, buf_cap) {{
        const entries = [];
        for (const [k, v] of Object.entries(env)) {{
          if (typeof v === "string") {{
            entries.push(k + "=" + v);
          }}
        }}
        if (entries.length === 0) return 0;
        const payload = entries.join("\0") + "\0\0";
        const bytes = encoder.encode(payload);
        if (bytes.length > buf_cap) {{
          return bytes.length;
        }}
        new Uint8Array(memory.buffer, buf_ptr, bytes.length).set(bytes);
        return bytes.length;
      }},
    }};

    // -- instantiate and run --

    const importObject = {{
      env: {{ memory }},
      wasi_snapshot_preview1: wasi,
      taida_host,
    }};

    const instance = await WebAssembly.instantiate(WASM, importObject);

    // If the module exports its own memory, use that instead
    if (instance.exports.memory) {{
      memory = instance.exports.memory;
    }}

    if (instance.exports._start) {{
      instance.exports._start();
    }}

    // Log stderr to console
    if (stderrChunks.length > 0) {{
      const errBytes = concat(stderrChunks);
      console.error(decoder.decode(errBytes));
    }}

    // Return stdout as response
    const outBytes = concat(stdoutChunks);
    return new Response(decoder.decode(outBytes), {{
      headers: {{ "content-type": "text/plain; charset=utf-8" }},
    }});
  }},
}};

function concat(arrays) {{
  let total = 0;
  for (const a of arrays) total += a.length;
  const result = new Uint8Array(total);
  let offset = 0;
  for (const a of arrays) {{
    result.set(a, offset);
    offset += a.length;
  }}
  return result;
}}
"#,
        stem = stem,
        wasm_filename = wasm_filename,
    )
}
