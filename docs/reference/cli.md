# Taida CLI リファレンス

> 更新日: 2026-03-05  
> 実装正本: `src/main.rs`

このページは Taida CLI の単一リファレンスです。  
コマンド仕様は実装 (`src/main.rs`) に合わせています。

---

## 実行モード

### 1. REPL

```bash
taida
taida --no-check
```

- 引数なしで REPL を起動します。
- `--no-check` を付けると REPL 内の型チェックをスキップします。

### 2. ファイル実行（Interpreter）

```bash
taida <FILE>
taida --no-check <FILE>
```

- サブコマンド以外の第1引数はファイルパスとして扱われます。
- `--no-check` で型チェックをスキップできます。

### 3. 共通フラグ

`--no-check` はグローバルに受理され、サブコマンド振り分け前に除去されます。

- 実際に効果がある: `taida <FILE>`, `taida` (REPL), `taida build`, `taida compile`(互換), `taida transpile`(互換)
- 現状効果がない: `graph`, `verify`, `inspect`, `init`, `deps`, `install`, `update`, `doc`, `todo`, `lsp`
- `--help` / `-h` はトップレベルヘルプを表示します。
- `--version` / `-V` はバージョンを表示します。

### 4. トップレベルヘルプ

```bash
taida --help
taida -h
taida help
```

挙動:
- 使用方法、主要サブコマンド、グローバルフラグを表示します。
- exit code は `0` です。
- `taida graph --help` のようなサブコマンド個別ヘルプへの導線を含みます。

---

## コマンド一覧

| コマンド | 用途 |
|---|---|
| `build` | 成果物生成の主コマンド（`--target js|native`） |
| `compile` | 非推奨互換: `build --target native` の alias |
| `transpile` | 非推奨互換: `build --target js` の alias |
| `todo` | `TODO[...]` 抽出と集計 |
| `check` | parse/type/verify(error-coverage) の静的診断 |
| `graph` | グラフ抽出 / summary / query |
| `verify` | 構造検証 |
| `inspect` | summary + graph統計 + verify 一括表示 |
| `init` | プロジェクト雛形生成 |
| `deps` | 依存解決 + install + lockfile（strict） |
| `install` | 依存インストール + lockfile生成 |
| `update` | リモート優先で依存更新 + lockfile更新 |
| `doc generate` | doc comments から Markdown 生成 |
| `lsp` | LSP サーバー起動（stdio） |

---

## `taida build`

```bash
taida build [--target js|native] [--release] [--diag-format text|jsonl] [-o OUTPUT] [--entry ENTRY] <PATH>
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--target <js\|native>` | - | 生成ターゲット（既定: `js`） |
| `--output <PATH>` / `--outdir <DIR>` | `-o` | 出力先（file入力時はファイル、dir入力時はディレクトリ） |
| `--entry <PATH>` | - | Native + dir入力時のエントリ上書き（既定: `main.td`） |
| `--release` | `-r` | TODO/Stub 残存時に失敗 |
| `--diag-format <text\|jsonl>` | - | 診断出力形式（既定: `text`） |

挙動:
- `<PATH>` は file/dir を受理します。
- `--target js`:
  - file入力: 単一 `.mjs` を生成（既定 `src/foo.td -> src/foo.mjs`）
  - dir入力: `.td` を再帰収集して `.mjs` を出力（既定出力は `<PATH>` 親の `dist/`）
  - dir入力で `package.json` が無ければ `{ "type": "module" }` を生成します。
- `--target native`:
  - file入力: そのファイルをエントリに Native バイナリ生成（既定 `src/foo.td -> src/foo`）
  - dir入力: 既定エントリ `<PATH>/main.td`、`--entry` で上書き可能
- 既定では parse + type check を実行します（`--no-check` で type check をスキップ）。
- `--release` では `TODO` / `Stub` を検出すると終了します（file入力時は import 依存も再帰走査）。
- `--diag-format jsonl` では compile 診断を `taida.diagnostic.v1` JSONL で出力します（parse/type/verify/codegen/io）。

---

## `taida compile`（非推奨互換）

```bash
taida compile [--release] [--diag-format text|jsonl] [-o OUTPUT] <FILE_OR_DIR>
```

挙動:
- `taida build --target native ...` の alias です。
- 実行時に deprecation warning を stderr 出力します。
- warning 文言:  
  `Warning: \`taida compile\` is deprecated and will be removed after a.1. Use \`taida build --target native\`.`

---

## `taida transpile`（非推奨互換）

```bash
taida transpile [--release] [--diag-format text|jsonl] [-o OUTPUT] <FILE_OR_DIR>
```

挙動:
- `taida build --target js ...` の alias です。
- 実行時に deprecation warning を stderr 出力します。
- warning 文言:  
  `Warning: \`taida transpile\` is deprecated and will be removed after a.1. Use \`taida build --target js\`.`

---

## `taida todo`

```bash
taida todo [--format text|json] [PATH]
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--format <FORMAT>` | `-f` | `text` (既定) / `json` |

挙動:
- `PATH` 省略時は `.` を対象にします。
- ディレクトリ指定時は `.td` を再帰収集します。
- JSON 出力は `total`, `todos`, `byId`, `byFile` を返します。

---

## `taida check`

```bash
taida check [--json] <PATH>
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--json` | - | 機械可読 JSON (`schema: taida.check.v1`) で出力 |

挙動:
- `<PATH>` はファイルまたはディレクトリ指定に対応します（ディレクトリ時は `.td` を再帰収集）。
- parse/type/verify(`error-coverage`) を実行し、`summary.total/errors/warnings/info/files` を出力します。
- `errors > 0` の場合は exit code `1`、それ以外は `0` です。

### Front Gate（仕様の正門）としての役割

`taida check` は単なる型検査器ではなく、**仕様の正門（front gate）** として機能します。

- **未実装機能の reject 口**: 言語仕様として確定しているが未実装の機能、または明示的に廃止された機能は、`taida check` が backend（Interpreter / JS / Native）に到達する前に拒否します。これにより backend ごとに別々の壊れ方をすることを防ぎます。
- **確定済み意味論の強制**: 関数オーバーロード禁止（`[E1501]`）、旧 `_` 部分適用禁止（`[E1502]`）、TypeDef 部分適用禁止（`[E1503]`）、パイプライン外 `Mold[_]()` 直接束縛禁止（`[E1504]`）など、確定済みの意味論制約を checker が強制します。
- **`--no-check` との関係**: `--no-check` で型チェックをスキップしても、parser error（`E01xx` / `E02xx`）は回避できません。checker が拒否する構文（`E13xx` / `E14xx` / `E15xx`）は `--no-check` でバイパスされますが、その場合 backend での挙動は未定義です。
- **診断コード**: `taida check` が発行する全ての診断コードは `docs/reference/diagnostic_codes.md` に定義されています。AI ツールはコードを安定識別子として利用できます。

---

## `taida graph`

```bash
taida graph [--type TYPE] [--format FORMAT] <PATH>
taida graph summary [--type TYPE] [--format FORMAT] <PATH>
taida graph query --type TYPE --query EXPR <PATH>
taida graph --help
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--type <TYPE>` | `-t` | `dataflow` / `module` / `type-hierarchy` / `error` / `call` |
| `--format <FORMAT>` | `-f` | `text` / `json` / `mermaid` / `dot` |
| `--query <EXPR>` | - | `query` サブコマンド用 |

挙動:
- 現状の `<PATH>` は単一ファイル入力です。
- 無効な `--type` / `--format` はエラー終了します。
- `summary` は構造サマリ(JSON文字列)を出力します。
- `--help` / `-h` は graph 専用の usage / type / format 一覧を表示し、exit code `0` で終了します。

---

## `taida verify`

```bash
taida verify [--check CHECK] [--format FORMAT] <PATH>
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--check <CHECK>` | `-c` | 実行チェック（複数指定可） |
| `--format <FORMAT>` | `-f` | `text` (既定) / `json` / `jsonl` / `sarif` |

`<PATH>` はファイルまたはディレクトリ指定に対応します。  
ディレクトリ指定時は `.td` を再帰走査します。

### `--check` の値

| チェック | 重要度 | 概要 |
|---|---|---|
| `error-coverage` | error | throw サイトのカバレッジ検証 |
| `no-circular-deps` | error | モジュール循環依存検出 |
| `dead-code` | warning | 到達不能関数検出 |
| `type-consistency` | error | 型階層の循環検出 |
| `unchecked-division` | warning | 除算安全性チェック枠（現行構文では通常 finding なし） |
| `direction-constraint` | error | 単一方向制約（`=>`/`<=`, `]=>`/`<=[`） |
| `unchecked-lax` | warning | Lax 未検査利用検出 |
| `naming-convention` | warning | 命名規約違反検出 |

注:
- 不明な check 名は `Unknown check` の warning として出力されます。
- `--severity` は現状未実装です。
- `--format jsonl` では finding 行 + summary 行を出力し、`ERROR` finding がある場合は exit code `1` になります。

---

## `taida inspect`

```bash
taida inspect [--format text|json|sarif] <PATH>
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--format <FORMAT>` | `-f` | `text` (既定) / `json` / `sarif` |

挙動:
- 現状の `<PATH>` は単一ファイル入力です。
- `text`: summary + 各グラフ統計 + verify結果を表示します。
- `json`: `summary` と `verification`（SARIF）を1つの JSON にまとめて出力します。
- `sarif`: verify 結果のみを SARIF で出力します。

---

## `taida init`

```bash
taida init [DIR]
```

挙動:
- `DIR` 省略時はカレントディレクトリに初期化します。
- `packages.tdm` が既にある場合は失敗します。
- `packages.tdm`, `main.td`（未存在時）, `.taida/` を作成します。

---

## `taida deps`

```bash
taida deps
```

挙動:
- `packages.tdm` をカレントから親方向に探索します。
- 依存解決エラーが1件でもある場合、install/lockfile更新を行わず失敗します（strict fail-fast）。
- 成功時は `.taida/deps` と `.taida/taida.lock` を更新します。

---

## `taida install`

```bash
taida install
```

挙動:
- `packages.tdm` をカレントから親方向に探索します。
- 解決できた依存を install し、`.taida/taida.lock` を生成/更新します。
- 一部依存が解決不能でも lockfile 生成は試行し、最後に終了コード1を返します。

---

## `taida update`

```bash
taida update
```

挙動:
- `install` と同様ですが、世代解決時にリモート優先モードで更新確認します。
- 解決できた依存を再 install し、`.taida/taida.lock` を更新します。

---

## `taida publish`

```bash
taida publish [--label LABEL] [--dry-run]
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--label <LABEL>` | - | publish version の末尾にラベルを付与します（例: `@a.4.rc`） |
| `--dry-run` | - | 実際の変更を行わず、計画だけ表示します |

挙動:
- `taida auth login` 済みであることを要求します（author 名の解決に使用）。
- working tree が clean であることを要求します（未コミットの変更がある場合はエラー）。
- `packages.tdm` と git tag (`<version>`) から次の publish version を決定します。
- `packages.tdm` の `<<<@...` を決定した exact version に更新します。
- git commit + tag (`<version>`) + push origin を実行します。
- 成功後、`taida-community/proposals` への登録用 URL を表示します。

---

## `taida doc generate`

```bash
taida doc generate [-o OUTPUT] <PATH>
```

| オプション | 短縮 | 説明 |
|---|---|---|
| `--output <PATH>` | `-o` | 出力先（省略時は標準出力） |

挙動:
- `<PATH>` はファイルまたはディレクトリ指定に対応します。
- ディレクトリ指定時は `.td` を再帰収集し、doc comments から Markdown を連結生成します。
- 現状は Markdown 出力のみです。

---

## `taida lsp`

```bash
taida lsp
```

挙動:
- Tokio runtime 上で LSP サーバーを起動します（stdio transport）。
