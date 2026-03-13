# Taida Lang -- FIX_PROGRESS

> 作成日: 2026-03-13
> 基準: `.dev/FIX_LIST.md` (FL-1〜FL-30, BE-WASM-1〜3, BE-TEST-1, N-1〜N-75)
> 目標: `@a.4.beta` リリース

## 運用方針

- FIX_LIST.md が「発見と詳細記述」、本ファイルが「修正の進捗管理」
- 各タスクの状態: `TODO` → `IN_PROGRESS` → `DONE` → `VERIFIED`
- `VERIFIED` = 修正 + テスト追加 + `cargo test` pass
- Phase 順に上から消化する。Phase 内は優先度順
- ブロッカー（他タスクへの依存）は `blocked_by` で明示

---

## Phase 0: Release Infrastructure (beta 前提条件)

リリースパイプライン自体の修正。コードの品質以前にパイプラインが正しくなければ beta を出せない。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 1 | FL-30 | LICENSE ファイル追加 | `DONE` | `/LICENSE` , `Cargo.toml` | — |
| 2 | FL-24 | release gate job に `--locked` 追加 | `DONE` | `.github/workflows/release.yml:86` | — |
| 3 | FL-19 | release workflow タグ検証 regex を仕様に合わせる | `DONE` | `.github/workflows/release.yml:45` | — |
| 4 | FL-18 | taida.dev/install.sh が最新 release を取得することの検証 + ドキュメント整合性 | `DONE` | `README.md`, `.dev/taida-logs/RELEASE_RUNBOOK.md` | — |

---

## Phase 1: Native Crash Blockers (segfault / silent corruption)

ユーザーコードで native backend が crash する問題。beta では native を advertise するため必須。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 5 | FL-8 | Native template literal `${...}` 補間が壊れている | `DONE` | `src/codegen/lower.rs` | — |
| 6 | FL-16 | Native untyped param 上の文字列 `+` が数値加算に lower → segfault | `DONE` | `src/codegen/lower.rs` | — |
| 7 | FL-21 | 未定義変数が checker 無診断 → native で silent `0` | `DONE` | `src/types/checker.rs` | — |
| 8 | FL-22 | 未知 method call が checker 無診断 → native で segfault | `DONE` | `src/types/checker.rs`, `src/types/checker_methods.rs` | — |
| 9 | FL-23 | non-function call が checker 無診断 → native で segfault | `DONE` | `src/types/checker.rs` | — |
| 10 | FL-9 | realloc() NULL チェック欠落 (26箇所) | `DONE` | `src/codegen/native_runtime.c` | — |
| 11 | FL-10 | mkdir_p の malloc NULL チェック + strcpy | `DONE` | `src/codegen/native_runtime.c` | — |
| 12 | FL-11 | emit.rs 未知 runtime 関数で panic → Result に変更 | `DONE` | `src/codegen/emit.rs` | — |

---

## Phase 2: Type Checker Front Gate

型チェッカーの欠落を埋める。Phase 1 の FL-21/22/23 とは別の、型注釈レベルの問題。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 13 | FL-1 | 戻り値注釈 `=> :T` が型検査で強制されない | `DONE` | `src/types/checker.rs` | — |
| 14 | FL-2 | Named type の未定義 field access が無診断 | `DONE` | `src/types/checker.rs` | — |
| 15 | FL-3 | 条件分岐の型検査が先頭 arm しか見ない | `DONE` | `src/types/checker.rs` | — |
| 16 | FL-4 | 比較・論理・単項演算子のオペランド型検証が欠落 | `DONE` | `src/types/checker.rs` | — |

---

## Phase 3: JS Backend 品質

JS codegen / runtime の出力品質。beta でユーザーが最も多く触る backend。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 17 | FL-12 | テンプレートリテラルのバッククォート未エスケープ | `DONE` | `src/js/codegen.rs` | — |
| 18 | FL-13 | HashMap/Set toString() の文字列エスケープ欠落 | `DONE` | `src/js/runtime.rs` | — |
| 19 | FL-14 | Div/Mod 浮動小数点検出の `String().includes('.')` → `Number.isInteger()` | `DONE` | `src/js/runtime.rs` | — |

---

## Phase 4: Test Harness / Parity Gate

テストインフラの信頼性。CI が安定しなければ修正の検証ができない。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 20 | FL-7 | Native build の .o 競合（並列 build race） | `DONE` | `src/codegen/driver.rs` | — |
| 21 | BE-TEST-1 | parity.rs の native build lock timeout | `DONE` | `tests/parity.rs`, `tests/native_compile.rs` | FL-7 |
| 22 | FL-17 | numbered examples を native parity gate に追加 | `DONE` | `tests/parity.rs` or `tests/run_backend_parity.sh` | FL-8, FL-16 |

---

## Phase 5: Package Manager 健全性

beta で `taida install` / `taida publish` を使わせる場合に必要。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 23 | FL-5 | publish が remote tag を見ず半壊状態になる | `DONE` | `src/pkg/publish.rs`, `src/main.rs` | — |
| 24 | FL-15 | manifest.rs の version unwrap() → panic | `DONE` | `src/pkg/manifest.rs` | — |
| 25 | FL-20 | `taida install` が lockfile を読まず再解決する | `DONE` | `src/main.rs`, `src/pkg/resolver.rs` | — |

---

## Phase 6: Security / Auth

認証トークンの取り扱い。beta で auth 機能を公開する場合に必要。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 26 | FL-25 | auth token のパーミッション 0o644 → 0o600 | `DONE` | `src/auth/token.rs` | — |
| 27 | FL-6 | auth token パスが HOME 固定 → USERPROFILE fallback | `DONE` | `src/auth/token.rs` | — |
| 28 | FL-26 | タイムスタンプ生成の Unix `date` 依存 → 純 Rust 実装 | `DONE` | `src/auth/token.rs` | — |

---

## Phase 7: Windows 互換性

Windows ビルドは release matrix に含まれている。beta で Windows を advertise する場合に必要。
advertise しない場合は `Known Limitations` として release notes に記載し、Phase を延期可。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 29 | FL-27 | native/wasm build の cc/which/wasm-ld が Unix 前提 | `DONE` | `src/codegen/driver.rs` | — |
| 30 | FL-28 | JS stdin が `/dev/stdin` ハードコード | `DONE` | `src/js/runtime.rs` | — |
| 31 | FL-29 | store/provider のホームディレクトリ fallback が `/tmp` | `DONE` | `src/pkg/store.rs`, `src/pkg/provider.rs` | FL-6 |

---

## Phase 8: WASM Backend

WASM profile の意味論不一致。beta では WASM は experimental 扱いで延期可。

| # | ID | 概要 | 状態 | 担当ファイル | blocked_by |
|---|-----|------|------|------------|-----------|
| 32 | BE-WASM-1 | TODO[T] が全 wasm profile で Molten に潰れる | `DONE` | `src/codegen/runtime_core_wasm.c` | — |
| 33 | BE-WASM-2 | `><` ゴリラが全 wasm profile で未実装 | `DONE` | `src/codegen/emit_wasm_c.rs`, `src/codegen/runtime_core_wasm.c` | — |
| 34 | BE-WASM-3 | wasm-full async の完了宣言と実装が不一致 | `DONE` | `.dev/taida-logs/DEV_PROGRESS_wasm_full.md` | — |

---

## Phase 9: Nice-to-have — Parser/Lexer (13件)

パニックリスク・エラーハンドリング・保守性の改善。現行で機能的には正しく動作している。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 35 | N-1 | `body_expr()` の非式 arm で `panic!()` | [P] | `VERIFIED` | `src/parser/ast.rs:307` |
| 36 | N-2 | `peek_at()` が `tokens.last().unwrap()` | [P] | `VERIFIED` | `src/parser/parser.rs:119` |
| 37 | N-3 | `advance()` 境界チェックなし | [P] | `VERIFIED` | `src/parser/parser.rs:124` |
| 38 | N-4 | `scan_number()` の直接インデックス | [P] | `VERIFIED` | `src/lexer/lexer.rs:363` |
| 39 | N-5 | バージョン文字列パーサーの状態機械が複雑 | [M] | `VERIFIED` | `src/parser/parser.rs:869-1031` |
| 40 | N-6 | Mold 判別のバックトラック | [L] | `VERIFIED` | `src/parser/parser_expr.rs:244-306` |
| 41 | N-7 | ブロック境界のインデント検出 | [L] | `VERIFIED` | `src/parser/parser.rs:1073-1148` |
| 42 | N-8 | `synchronize()` のスキップ範囲 | [L] | `VERIFIED` | `src/parser/parser.rs:221-231` |
| 43 | N-9 | import パス未知トークンのデバッグ文字列化 | [E] | `VERIFIED` | `src/parser/parser.rs:768-771` |
| 44 | N-10 | 不正エスケープがトークンに含まれたまま | [E] | `VERIFIED` | `src/lexer/lexer.rs:554-562` |
| 45 | N-11 | テンプレート文字列の不明エスケープがログなし | [E] | `VERIFIED` | `src/lexer/lexer.rs:606-609` |
| 46 | N-12 | `peek_at(offset)` にルックアヘッド上限なし | [L] | `VERIFIED` | `src/parser/parser.rs` |
| 47 | N-13 | 行継続の前処理が複雑 | [M] | `VERIFIED` | `src/parser/parser.rs:43-70` |

---

## Phase 10: Nice-to-have — Interpreter (18件)

unwrap パターン、silent default、テストコードの改善。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 48 | N-14 | `chars().nth(idx).unwrap()` (charAt) | [P] | `VERIFIED` | `src/interpreter/methods.rs:1055` |
| 49 | N-15 | `scopes.last_mut().unwrap()` (define) | [P] | `VERIFIED` | `src/interpreter/env.rs:43` |
| 50 | N-16 | `scopes.last_mut().unwrap()` (define_force) | [P] | `VERIFIED` | `src/interpreter/env.rs:57` |
| 51 | N-17 | async resolve の check-then-unwrap | [P] | `VERIFIED` | `src/interpreter/unmold.rs:43` |
| 52 | N-18 | async resolve の check-then-unwrap (別箇所) | [P] | `VERIFIED` | `src/interpreter/unmold.rs:109` |
| 53 | N-19 | テスト内の `panic!("Expected ...")` (11箇所) | [T] | `VERIFIED` | `src/interpreter/json.rs` |
| 54 | N-20 | 統合テストの `panic!("Expected Bool...")` | [T] | `VERIFIED` | `src/interpreter/os_eval.rs:2505,2538` |
| 55 | N-21 | `unreachable!()` にコメントなし | [M] | `VERIFIED` | `src/interpreter/mold_eval.rs:793` |
| 56 | N-22 | check-then-unwrap アンチパターン | [M] | `VERIFIED` | `src/interpreter/unmold.rs:43,109` |
| 57 | N-23 | JSON パース失敗時の silent default | [E] | `VERIFIED` | `src/interpreter/json.rs:254,260,264,274` |
| 58 | N-24 | `parent().unwrap_or(Path::new("."))` | [E] | `VERIFIED` | `src/interpreter/module_eval.rs:21,89` |
| 59 | N-25 | export シンボル不在時の silent fallback | [E] | `VERIFIED` | `src/interpreter/module_eval.rs:380` |
| 60 | N-26 | TODO コメント残存 (`]=>` channel) | [M] | `VERIFIED` | `src/interpreter/unmold.rs:263` |
| 61 | N-27 | TODO mold コメントが探索的実装を示唆 | [M] | `VERIFIED` | `src/interpreter/mold_eval.rs:1711-1738` |
| 62 | N-28 | "unexpected signal" にシグナル型が含まれない | [E] | `VERIFIED` | `src/interpreter/os_eval.rs` |
| 63 | N-29 | Tokio runtime の `expect()` | [P] | `VERIFIED` | `src/interpreter/eval.rs:139` |
| 64 | N-30 | Pipeline scope の RAII ガードなし | [L] | `VERIFIED` | `src/interpreter/control_flow.rs:32-55` |
| 65 | N-31 | `eval_mold()` の match arm が巨大 | [M] | `VERIFIED` | `src/interpreter/mold_eval.rs` |

---

## Phase 11: Nice-to-have — JS codegen (8件)

レジストリ・変数名・フォールバックの改善。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 66 | N-32 | 相互再帰検出の `position().unwrap()` | [P] | `VERIFIED` | `src/js/codegen.rs:176` |
| 67 | N-33 | テンプレートリテラル変換の限定 | [L] | `VERIFIED` | `src/js/codegen.rs:2039-2047` |
| 68 | N-34 | Mold フィールドレジストリの更新タイミング | [L] | `VERIFIED` | `src/js/codegen.rs:882-883` |
| 69 | N-35 | manifest 読み込み失敗時の silent fallback | [E] | `VERIFIED` | `src/js/codegen.rs:1109-1115` |
| 70 | N-36 | Pipeline の `__p` ハードコード変数名 | [L] | `VERIFIED` | `src/js/codegen.rs:2054-2064` |
| 71 | N-37 | TODO mold の `__type: 'TODO'` マーカー名 | [M] | `VERIFIED` | `src/js/codegen.rs:1843`, `src/js/runtime.rs:618` |
| 72 | N-38 | HashMap/Set toString() フォーマット不統一 | [L] | `VERIFIED` | `src/js/runtime.rs:1606-1612,1689-1691` |
| 73 | N-39 | 型引数不足時の `"undefined"` フォールバック | [E] | `VERIFIED` | `src/js/codegen.rs:946-951` |

---

## Phase 12: Nice-to-have — Native codegen (5件)

strcpy パターン、フォールバック、コメント整理。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 74 | N-40 | `strcpy()` 使用（mkdir_p 以外の3箇所） | [P] | `VERIFIED` | `src/codegen/native_runtime.c` |
| 75 | N-41 | `unwrap_or(Path::new("."))` 等 (15箇所) | [E] | `VERIFIED` | `src/codegen/driver.rs` |
| 76 | N-42 | TODO コメント残存 (unmold channel) | [M] | `VERIFIED` | `src/codegen/native_runtime.c:4896` |
| 77 | N-43 | malloc NULL チェックの一貫性不足 | [E] | `VERIFIED` | `src/codegen/native_runtime.c` |
| 78 | N-44 | ABI テーブル保守性 | [M] | `VERIFIED` | `src/codegen/emit.rs:1512` |

---

## Phase 13: Nice-to-have — CLI/pkg/auth (14件)

unwrap チェーン、silent エラー、パターン統一。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 79 | N-45 | REPL `flush().unwrap()` | [P] | `VERIFIED` | `src/main.rs:4198` |
| 80 | N-46 | device_flow のエラー詳細消失 | [E] | `VERIFIED` | `src/auth/device_flow.rs:58,137` |
| 81 | N-47 | ネストした `unwrap_or_else` | [E] | `VERIFIED` | `src/main.rs:1316` |
| 82 | N-48 | 依存収集のパス解決フォールバック | [E] | `VERIFIED` | `src/main.rs:2922,2927` |
| 83 | N-49 | `canonicalize()` のフォールバック | [E] | `VERIFIED` | `src/main.rs:1343-1344` |
| 84 | N-50 | パストラバーサルの理論的リスク | [E] | `VERIFIED` | `src/pkg/resolver.rs:55` |
| 85 | N-51 | import パス解決の多段 `unwrap_or` チェーン | [E] | `VERIFIED` | `src/main.rs:1408-1422` |
| 86 | N-52 | `option_env!` の `unwrap_or` | [E] | `VERIFIED` | `src/main.rs:25` |
| 87 | N-53 | `SystemTime` の `expect()` | [P] | `VERIFIED` | `src/main.rs:1505-1506` |
| 88 | N-54 | LSP 用 Tokio runtime `expect()` | [P] | `VERIFIED` | `src/main.rs:4187` |
| 89 | N-55 | エラーハンドリングパターンの不統一 | [M] | `VERIFIED` | `src/main.rs` |
| 90 | N-56 | `init` のディレクトリ作成エラー無視 | [E] | `VERIFIED` | `src/main.rs:3745-3751` |
| 91 | N-57 | ステージングファイル削除エラー無視 | [E] | `VERIFIED` | `src/main.rs:1582-1585` |
| 92 | N-58 | トークンファイルのパーミッション未設定 | [E] | `VERIFIED` | `src/auth/token.rs` |

---

## Phase 14: Nice-to-have — Type checker/Tests (17件)

テストの assert 改善、catch-all の安全性、カバレッジ拡充。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 93 | N-59 | テスト内 `panic!` → `assert!` に統一 | [T] | `VERIFIED` | `src/types/checker_tests.rs:684` |
| 94 | N-60 | レジストリ取得の `unwrap()` 検証なし | [T] | `VERIFIED` | `src/types/checker_tests.rs:155,167,176` |
| 95 | N-61 | `get_type_fields().unwrap()` | [T] | `VERIFIED` | `src/types/types.rs:463` |
| 96 | N-62 | `!lines.is_empty()` のみの弱いアサーション | [T] | `VERIFIED` | `tests/todo_cli.rs:570` |
| 97 | N-63 | `as_array().unwrap()` で JSON 構造を仮定 | [T] | `VERIFIED` | `tests/todo_cli.rs:1353` |
| 98 | N-64 | `check_program()` の `_ => {}` catch-all | [L] | `VERIFIED` | `src/types/checker.rs:814` |
| 99 | N-65 | `check_statement()` の `_ => {}` catch-all | [L] | `VERIFIED` | `src/types/checker.rs:1798-1819` |
| 100 | N-66 | 未知 receiver 型のメソッドチェックスキップ | [L] | `VERIFIED` | `src/types/checker_methods.rs:86-90` |
| 101 | N-67 | `unwrap_or(Type::Unknown)` で型精度低下 | [E] | `VERIFIED` | `src/types/checker.rs` |
| 102 | N-68 | エラーコードの付与が不統一 | [M] | `VERIFIED` | `src/types/checker.rs` |
| 103 | N-69 | 循環型依存のテストなし | [T] | `VERIFIED` | `src/types/checker_tests.rs` |
| 104 | N-70 | ジェネリック型制約のテストカバレッジ限定 | [T] | `VERIFIED` | `src/types/checker_tests.rs` |
| 105 | N-71 | `@[]` 型パラメータ推論の負テストなし | [T] | `VERIFIED` | `src/types/checker_tests.rs` |
| 106 | N-72 | 型エラー報告で行番号が欠落 | [T] | `VERIFIED` | `tests/typecheck_examples.rs:42` |
| 107 | N-73 | Optional/Result 再設計の migration marker なし | [T] | `VERIFIED` | `src/types/` |
| 108 | N-74 | `resolve_type()` がキャッシュしない | [L] | `VERIFIED` | `src/types/types.rs:326-372` |
| 109 | N-75 | スコープスタック非空の不変条件が未ドキュメント | [M] | `VERIFIED` | `src/types/checker.rs:699,721,733` |

---

## Phase 15: Nice-to-have — WASM/Runtime Hardening (6件)

poly_add 文字列対応、int_mold_str 負数修正、delete_token TOCTOU、emit.rs panic 除去。

| # | ID | 概要 | 分類 | 状態 | 担当ファイル |
|---|-----|------|------|------|------------|
| 110 | NTH-1 | `taida_poly_add` に float 判定追加 (native) | [E] | `DONE` | `src/codegen/native_runtime.c` |
| 111 | NTH-2 | `delete_token` の TOCTOU 解消 | [E] | `DONE` | `src/auth/token.rs` |
| 112 | NTH-3 | `emit.rs` の残存 `panic!` を Result 伝播に変更 | [E] | `DONE` | `src/codegen/emit.rs:2212` |
| 113 | NTH-4 | wasm-min `taida_int_mold_str` 負数で OOB | [B] | `DONE` | `src/codegen/runtime_core_wasm.c` |
| 114 | NTH-5 | wasm `taida_poly_add` 文字列対応 | [E] | `DONE` | `src/codegen/runtime_core_wasm.c` |
| 115 | NTH-6 | wasm allowlist 肥大化 (NTH-5 解消で縮小) | [T] | `DONE` | `tests/wasm_full.rs`, `tests/wasm_wasi.rs` |

---

## 完了サマリ

### Must-fix (Phase 0〜8)

| Phase | 名称 | 件数 | 完了 | 残り |
|-------|------|------|------|------|
| 0 | Release Infrastructure | 4 | 4 | 0 |
| 1 | Native Crash Blockers | 8 | 8 | 0 |
| 2 | Type Checker Front Gate | 4 | 4 | 0 |
| 3 | JS Backend 品質 | 3 | 3 | 0 |
| 4 | Test Harness / Parity Gate | 3 | 3 | 0 |
| 5 | Package Manager 健全性 | 3 | 3 | 0 |
| 6 | Security / Auth | 3 | 3 | 0 |
| 7 | Windows 互換性 | 3 | 3 | 0 |
| 8 | WASM Backend | 3 | 3 | 0 |
| **小計** | | **34** | **34** | **0** |

### Nice-to-have (Phase 9〜14)

| Phase | 名称 | 件数 | 完了 | 残り |
|-------|------|------|------|------|
| 9 | Parser/Lexer | 13 | 13 | 0 |
| 10 | Interpreter | 18 | 18 | 0 |
| 11 | JS codegen | 8 | 8 | 0 |
| 12 | Native codegen | 5 | 5 | 0 |
| 13 | CLI/pkg/auth | 14 | 14 (VERIFIED) | 0 |
| 14 | Type checker/Tests | 17 | 17 (VERIFIED) | 0 |
| 15 | WASM/Runtime Hardening | 6 | 6 | 0 |
| **小計** | | **81** | **81** | **0** |

| | | **総計 115** | **115** | **0** |

---

## 意思決定ログ

> 決定日: 2026-03-13

### D-1: LICENSE の種別 (FL-30) → **MIT**

- 決定: MIT ライセンス
- 理由: `taida-human` CLI 拡張で収益化予定。コア (MIT) と商用拡張の境界を明確にする戦略
- 注意: MIT はコア言語・コンパイラに適用。`taida-human` は別ライセンス（商用）で提供

### D-2: install.sh の方針 (FL-18) → **taida.dev に搭載済み**

- 決定: `taida.dev/install.sh` が既に存在する。最新リリースを自動取得できることを検証する
- 作業: README / runbook の記述が `taida.dev/install.sh` を正しく指していることを確認。release workflow との連携を検証
- FL-18 の修正内容: `install.sh` の新規作成ではなく、taida.dev 側の install.sh が最新 GitHub Release を取得することの検証とドキュメント整合性確認

### D-3: beta で公開する機能スコープ → **Phase 0〜8 全て beta 必須**

- 決定: 全 Phase を beta リリースで完了する。Known Limitations による延期なし
- Phase 5 (Package Manager): beta 必須
- Phase 6 (Security/Auth): beta 必須
- Phase 7 (Windows): beta 必須
- Phase 8 (WASM): beta 必須

### D-4: Phase 2 (Type Checker) の beta 扱い → **beta 必須**

- 決定: FL-1〜FL-4 を全て beta で修正する
- Phase 1 (FL-21/22/23, native segfault 直結) と Phase 2 (FL-1〜FL-4, 型注釈レベル) の両方を beta で完了

---

## beta リリース必須要件

> D-3/D-4 決定により、全 Phase が beta 必須。延期・Known Limitations 扱いなし。

- [ ] Phase 0 完了（リリースインフラ — MIT LICENSE, release workflow, install.sh 検証）
- [ ] Phase 1 完了（native crash 0 件）
- [ ] Phase 2 完了（型チェッカー強化 — FL-1〜FL-4）
- [ ] Phase 3 完了（JS 品質）
- [ ] Phase 4 完了（テスト安定）
- [ ] Phase 5 完了（Package Manager 健全性）
- [ ] Phase 6 完了（Security / Auth）
- [ ] Phase 7 完了（Windows 互換性）
- [ ] Phase 8 完了（WASM Backend）
- [ ] `cargo test` 全 pass
- [ ] `./tests/run_backend_parity.sh` pass
- [ ] `./tests/e2e_smoke.sh` pass

---

## 更新ログ

| 日付 | 内容 |
|------|------|
| 2026-03-13 | 初版作成。FL-1〜FL-30 + BE-WASM-1〜3 + BE-TEST-1 を Phase 0〜8 に登録 |
| 2026-03-13 | N-1〜N-75 を Phase 9〜14 に追加。意思決定セクション (D-1〜D-4) 追加 |
| 2026-03-13 | D-1〜D-4 全決定。MIT LICENSE、install.sh は taida.dev 搭載済み、Phase 0〜8 全て beta 必須 |
| 2026-03-13 | Phase 1 全完了 (FL-8,9,10,11,16,21,22,23)。poly_add 誤発火修正: 戻り値型 `:Int` からパラメータ型推論 |
| 2026-03-13 | Phase 2 全完了 (FL-1,2,3,4)。型チェッカー強化: 戻り値注釈強制、Named type field診断、全arm型検査、演算子型検証 |
| 2026-03-13 | Phase 3 全完了 (FL-12,13,14)。JS Backend 品質: テンプレートリテラルバッククォートエスケープ、HashMap/Set toString エスケープ、Div/Mod Number.isInteger 統一 |
| 2026-03-13 | Phase 4 全完了 (FL-7,BE-TEST-1,FL-17)。Test Harness: unique .o パスで並列 race 解消、build lock 除去、numbered examples native parity テスト追加 |
| 2026-03-13 | Phase 5 全完了 (FL-5,FL-15,FL-20)。Package Manager 健全性: publish remote tag 事前チェック+ロールバック、manifest unwrap 安全化、install lockfile ピン |
| 2026-03-13 | Phase 6 全完了 (FL-25,FL-6,FL-26)。Security/Auth: token パーミッション 0o600 強制、HOME/USERPROFILE/temp_dir フォールバック、純 Rust RFC3339 タイムスタンプ |
| 2026-03-13 | Phase 7 全完了 (FL-27,FL-28,FL-29)。Windows 互換性: which_command 抽象化+Windows コンパイラ検出+プラットフォーム別エラーメッセージ、stdin を process.stdin.fd に変更、store/provider フォールバックを std::env::temp_dir() に統一 |
| 2026-03-13 | Phase 8 全完了 (BE-WASM-1,BE-WASM-2,BE-WASM-3)。WASM Backend: TODO[T] pack を native と同一レイアウトで実装+generic_unmold に TODO 分岐追加、taida_gorilla を全 wasm profile に追加 (__builtin_trap)、wasm-full async ドキュメント訂正 |
| 2026-03-13 | 横断レビュー Should Fix 3件修正: taida_home_dir() を util.rs に移動、module_graph.rs の HOME 直接参照を統一、環境変数テストのロック共通化 |
| 2026-03-14 | Phase 15 全完了 (NTH-1〜NTH-6)。WASM/Runtime Hardening: native poly_add に float heuristic 追加、delete_token TOCTOU 解消、emit.rs panic を Result 伝播に変更、wasm generic_unmold に非ポインタ値ガード追加+str_to_int 符号なしオーバーフロー修正、wasm poly_add に文字列検出追加 (parity: wasm-full 42→51, wasm-wasi 24→27)、allowlist 縮小 |
| 2026-03-14 | Phase 9 全完了 (N-1〜N-13)。Parser/Lexer: body_expr panic メッセージ改善、peek_at/advance 境界チェック強化 (expect+saturating_add+min clamp)、scan_number SAFETY コメント、バージョンパーサー状態機械ドキュメント、Mold バックトラックコメント、ブロックパーサー/synchronize/行継続コメント補強、import パス未知トークンコメント、不正エスケープ error recovery コメント、テンプレート文字列不明エスケープにエラー報告追加 (通常文字列との一貫性)、peek_at ルックアヘッドドキュメント、lexer テスト2件追加 |
| 2026-03-14 | Phase 10 全完了 (N-14〜N-31)。Interpreter: charAt unwrap_or_default 化、env.rs scope 非空不変条件ドキュメント+expect メッセージ、unmold.rs check-then-unwrap を match 統合に変更、json.rs テスト panic→let-else 変換 (11箇所)、os_eval.rs テスト panic→unreachable 変換+signal_name ヘルパー追加で unexpected signal にシグナル型を含める、mold_eval.rs unreachable にコメント追加+TODO mold ドキュメント整理+eval_mold match 巨大さのドキュメント、json.rs silent default の哲学準拠コメント、module_eval.rs parent/export フォールバックコメント、eval.rs expect メッセージ改善、control_flow.rs pipeline scope RAII 不使用の理由ドキュメント |
| 2026-03-14 | Phase 11 全完了 (N-32〜N-39)。JS codegen: 相互再帰 position().unwrap() に SAFETY コメント+expect メッセージ、テンプレートリテラル変換の制限事項ドキュメント、Mold フィールドレジストリ更新タイミングコメント、manifest 読み込み失敗時に eprintln 警告追加 (silent fallback 解消)、Pipeline __p 変数の IIFE スコープ安全性ドキュメント、TODO mold __type マーカー命名規則コメント (codegen+runtime 両方)、HashMap/Set toString() フォーマットドキュメント、型引数不足時の undefined フォールバック理由ドキュメント |
| 2026-03-14 | Phase 12 全完了 (N-40〜N-44)。Native codegen: strcpy→memcpy/snprintf 変換 (3箇所)、driver.rs unwrap_or(Path) の安全性ドキュメント、native_runtime.c の stale TODO コメント更新、TAIDA_MALLOC マクロ導入+未チェック malloc 一括変換 (30箇所以上)、emit.rs ABI テーブル保守ガイドコメント追加 |
| 2026-03-14 | Phase 13 全完了 (N-45〜N-58)。CLI/pkg/auth: REPL flush を is_err+break に変更、device_flow エラー本文保存、unwrap_or_else フラット化、parent/canonicalize フォールバックドキュメント、resolver.rs パストラバーサル安全性コメント、import パス解決チェーンドキュメント、option_env コンパイル時解決ドキュメント、SystemTime/Tokio expect メッセージ改善、エラーハンドリング規約をファイル先頭に記述、init ディレクトリ作成エラーを warning 化、staging ファイル削除に NotFound ドキュメント、token.rs 非 Unix パーミッションドキュメント |
| 2026-03-14 | Phase 13 VERIFIED (N-45〜N-58)。Phase Gate 通過: cargo test 1504 pass / 0 fail、run_backend_parity.sh 66 pass / 0 fail、e2e_smoke.sh 73 pass / 0 fail |
| 2026-03-14 | Phase 14 VERIFIED (N-59〜N-75)。Phase Gate 通過: cargo test 1536 pass / 0 fail、run_backend_parity.sh 66 pass / 0 fail、e2e_smoke.sh 73 pass / 0 fail |
| 2026-03-14 | Phase 16 全完了 (R-01〜R-11, 9件)。横断レビュー指摘: utf8_decode_mold に len<=0 ガード追加、taida_safe_malloc で size==0 を 1 に正規化、parser advance() に debug_assert 追加、checker_tests 3件に具体的 assertion 追加、lax_result テストから None 許容除去、hashmap/set_to_string の strcat をオフセットベース memcpy に変換、charAt の unwrap_or_default を expect に変更、自明な N-XX コメント削除 (7件)、固定文字列 snprintf を memcpy に統一 (4箇所) |
| 2026-03-14 | Phase 14 全完了 (N-59〜N-75)。Type checker/Tests: test panic→assert 統一、unwrap→expect 変換 (checker_tests 3箇所+types.rs 1箇所+todo_cli 2箇所)、check_program/check_statement catch-all 意図ドキュメント、checker_methods 未知 receiver スキップ理由ドキュメント、unwrap_or(Type::Unknown) 設計規約をモジュール docstring に記述、エラーコード一覧ドキュメント、循環型依存/自己参照型テスト追加、ジェネリック制約エッジケーステスト追加、@[] 負テスト追加、typecheck_examples 行番号コメント、Optional/Result migration marker テスト追加、resolve_type キャッシュ不使用の理由ドキュメント、scope_stack 非空不変条件ドキュメント |
| 2026-03-14 | Phase 17a 全完了 (M-00,M-01,M-03,M-06,M-07,M-09,M-10,M-16)。native_runtime.c 整数オーバーフロー防御: taida_safe_mul/taida_safe_add ヘルパー導入、taida_pack_new に負値+オーバーフローガード、taida_hashmap_clone に NULL+cap ガード、taida_list_join に malloc NULL+len*sizeof オーバーフロー+total size_t 化+safe_add、taida_list_sort/sort_desc に len==0 早期リターン+safe_mul+NULL チェック、taida_str_alloc に len 上限ガード (SIZE_MAX-17)、taida_str_concat に la+lb safe_add。cargo test 1536 pass / 0 fail |
| 2026-03-14 | Phase 17b 全完了 (M-02,M-04,M-05,M-11,M-15)。native_runtime.c should-fix オーバーフロー防御: hashmap_new_with_cap に cap<=0 正規化+safe_mul/safe_add でスロット計算、bytes_new_filled に len 上限ガード+safe_mul/safe_add、list_push に cap*2 符号付きオーバーフロー検出+safe_mul/safe_add で realloc サイズ計算、socket_recv_exact に 256MB 上限、writeBytes/socketSend/socketSendAll/udpSendTo の Bytes 分岐に 256MB 上限。cargo test 1536 pass / 0 fail |
| 2026-03-14 | Phase 17c 全完了 (M-08,M-12,M-14)。native_runtime.c minor malloc 安全化: taida_sha256 Bytes 分岐に 256MB 上限ガード追加、taida_os_list_dir に capacity オーバーフロー検出+初期 malloc を TAIDA_MALLOC 化+realloc サイズに safe_mul 適用、NULL チェックなし raw malloc を TAIDA_MALLOC に統一 (async_ok/ok_tagged/err/json_parse_array/json_parse_object/exec_argv + 既存の手動 NULL+exit を TAIDA_MALLOC に統一: str_alloc/pack_new/closure_new/list_new/bytes_new_filled/list_join/list_sort/list_sort_desc/hashmap_clone/async_spawn)。I/O 系関数の graceful-degradation パターン (NULL→return error) は意図的に raw malloc を維持。cargo test 1536 pass / 0 fail |

---

## Phase 16: Phase 9-15 横断レビュー指摘 (9件)

Phase 9-15 のコードレビューで検出された修正項目。

### Must Fix

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| R-01 | critical | `taida_utf8_decode_mold` に `len < 0` ガード欠如 — 負値で巨大 malloc → OOM abort | `src/codegen/native_runtime.c` | `VERIFIED` |
| R-02 | major | `taida_safe_malloc` で `size == 0` のとき `malloc(0)` → NULL → exit(1) — `size = 1` に正規化 | `src/codegen/native_runtime.c` | `VERIFIED` |

### Should Fix

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| R-05 | minor | `parser.rs` `advance()` に `debug_assert!` 追加（防御的クランプが不正状態を隠す） | `src/parser/parser.rs` | `VERIFIED` |
| R-09 | minor | 新規テスト3件の assertion が弱い（パニックしないことだけ検証）— 具体的 assertion 追加 | `src/types/checker_tests.rs` | `VERIFIED` |
| R-10 | minor | `test_lax_result_current_behavior_stable` で `None` 許容が広すぎ — 除去 | `src/types/checker_tests.rs` | `VERIFIED` |

### Nice to Have

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| R-03 | minor | `hashmap_to_string` / `set_to_string` 内の `strcat` が残存 — 安全パターンに置換 | `src/codegen/native_runtime.c` | `VERIFIED` |
| R-04 | minor | `methods.rs` の `unwrap_or_default` を `expect("bounds checked above")` に変更 | `src/interpreter/methods.rs` | `VERIFIED` |
| R-06 | nit | 自明な N-XX コメント削除（情報量ゼロのもの） | 複数ファイル | `VERIFIED` |
| R-11 | nit | `snprintf` → `memcpy` に統一（固定文字列コピー） | `src/codegen/native_runtime.c` | `VERIFIED` |

---

## Phase 17: 巨大 malloc / 整数オーバーフロー防御 (16件)

native_runtime.c の malloc サイズ計算における整数オーバーフロー・負値・巨大値の網羅的防御。
横断的に `taida_safe_mul` / `taida_safe_add` ヘルパーを導入し、全箇所に適用する。

### Phase 17a: インフラ + Critical (Must Fix)

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| M-00 | infra | `taida_safe_mul` / `taida_safe_add` ヘルパー導入 | `src/codegen/native_runtime.c` | `DONE` |
| M-01 | critical | `taida_pack_new`: 負の `field_count` で整数オーバーフロー → ヒープ破壊 | `src/codegen/native_runtime.c` | `DONE` |
| M-03 | critical | `taida_hashmap_clone`: `cap` オーバーフロー + NULL チェック欠如 | `src/codegen/native_runtime.c` | `DONE` |
| M-06 | critical | `taida_list_join`: malloc に NULL チェックなし + `len * sizeof` オーバーフロー | `src/codegen/native_runtime.c` | `DONE` |
| M-07 | critical | `taida_list_sort` / `sort_desc`: malloc NULL チェックなし + オーバーフロー | `src/codegen/native_runtime.c` | `DONE` |
| M-09 | critical | `taida_str_alloc`: `len + 17` の size_t オーバーフロー → ヒープ破壊 | `src/codegen/native_runtime.c` | `DONE` |
| M-10 | critical | `taida_str_concat`: `la + lb` の size_t オーバーフロー → ヒープ破壊 | `src/codegen/native_runtime.c` | `DONE` |
| M-16 | critical | `taida_list_join`: total の int64_t オーバーフロー → ヒープ破壊 | `src/codegen/native_runtime.c` | `DONE` |

### Phase 17b: Should Fix

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| M-02 | major | `taida_hashmap_new_with_cap`: `cap` 上限ガード欠如 | `src/codegen/native_runtime.c` | `DONE` |
| M-04 | major | `taida_bytes_new_filled`: `len` 上限ガード欠如（巨大正値で OOM） | `src/codegen/native_runtime.c` | `DONE` |
| M-05 | major | `taida_list_push`: realloc の `cap * 2` オーバーフロー | `src/codegen/native_runtime.c` | `DONE` |
| M-11 | major | `taida_os_socket_recv_exact`: `size` 上限ガード欠如 | `src/codegen/native_runtime.c` | `DONE` |
| M-15 | major | Bytes 系 send 関数: `len` 上限ガード欠如 | `src/codegen/native_runtime.c` | `DONE` |

### Phase 17c: Nice to Have

| ID | severity | 概要 | ファイル | 状態 |
|----|----------|------|---------|------|
| M-08 | minor | `taida_sha256` Bytes 分岐: 上限ガード追加 | `src/codegen/native_runtime.c` | `DONE` |
| M-12 | minor | `taida_os_list_dir`: capacity オーバーフロー防御 | `src/codegen/native_runtime.c` | `DONE` |
| M-14 | minor | NULL チェックなしの raw malloc を TAIDA_MALLOC に統一 | `src/codegen/native_runtime.c` | `DONE` |

---

## Nice to Have (横断レビューで検出、beta 後に対応)

NTH-1〜NTH-6 は Phase 15 に統合済み。
