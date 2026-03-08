/// コンパイルドライバ — .td → パース → IR → CLIF → .o → バイナリ
///
/// cranelift-object で .o を出力し、システムリンカでバイナリを生成。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::emit::Emitter;
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

/// 単一 .td ファイルを .o にコンパイル（リンクなし）
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
