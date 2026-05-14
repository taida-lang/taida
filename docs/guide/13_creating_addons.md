# インゴット内でのアドオン作成

> **PHILOSOPHY.md — IV.** キモい言語だ。だが、書くのも読むのもAI。眺めるのが人間の仕事。

**インゴット (ingot)** はパッケージの単位です。**アドオン (addon)** は
インゴットの中に置かれるネイティブローダー層であり、Rust の `cdylib` と
`native/addon.toml` の組み合わせで構成されます。マニフェストには ABI、
パッケージ名、関数表、任意のプレビルド配布メタデータが pin されます。
マニフェストファイル名は `addon.toml` のままで、パッケージ系 CLI
コマンドは `taida ingot` 配下に集約されています。

本ガイドはアドオンの *作者* を対象としています。マニフェストスキーマと
前方互換ポリシーは [アドオンマニフェスト](../reference/addon_manifest.md)、
`taida ingot publish` の CLI 契約は [CLI リファレンス](../reference/cli.md)
を参照してください。

### バックエンドポリシー (どこでアドオンが動くか)

アドオンが対応するバックエンドの一覧は次のとおりです。

| バックエンド | 状態 | インポート時の挙動 |
|--------------|------|--------------------|
| インタプリタ | **対応** | アドオンファサードが動的モジュールとして動作し、`feature = "native"` でビルドされたインタプリタ (既定ビルド) では cdylib 関数が `dlopen` 経由でディスパッチされます。 |
| ネイティブ (AOT) | **対応** | アドオンファサードはビルド時に静的解析されてサマリ化され、FuncDef は IR 関数に、pack / scalar / list / template バインディングはモジュール初期化パスに置き換えられます。 |
| JS トランスパイラ | **非対応** | 現状 JS 側にアドオンディスパッチャは存在せず、インポートは決定的なエラーを返します。 |
| WASM (`wasm-min` / `wasm-wasi` / `wasm-edge`) | **非対応** | これらのプロファイルはアドオンディスパッチャを公開していません。将来拡張する場合もインタプリタ / ネイティブと同じ静的解析器を再利用する設計のため、作者がファサードを書き直す必要はありません。アドオンディスパッチに現状対応している WASM プロファイルは `wasm-full` のみです — 詳細は [WASM プロファイル](../reference/wasm_profiles.md) を参照してください。 |

インタプリタとネイティブを対象とする作者は、ファサードファイルを 2
通り書く必要はありません。同じ `taida/<stem>.td` が両方で動作します。
インタプリタはファサードの生環境スナップショットに対してユーザー
インポートを解決し、ネイティブバックエンドは静的解析で抽出した
ファサードサマリに対して解決します。一方のパスで受理される構文は
もう一方でも受理されます (詳細は次節
[ネイティブバックエンドがファサード内で理解する構文](#ネイティブバックエンドがファサード内で理解する構文)
を参照)。

### ネイティブバックエンドがファサード内で理解する構文

ファサードローダーは、最上位に次の構文要素を許容します。

- **エイリアス** — `FacadeName <= lowercaseAddonFn`。`lowercaseAddonFn`
  はマニフェストの `[functions]` 表に列挙された関数名です。
- **pack リテラル** — `FacadeName <= @(field <= value, ...)`。
- **スカラ / リスト / 算術 / テンプレートバインディング** —
  `N <= 0`、`msg <= "hello"`、`greet <= \`hi, ${who}\``、算術式、関数
  呼び出し、メソッド呼び出し、フィールドアクセス、モールド / 型の
  インスタンス化。
- **FuncDef** — `Name args = body => :Type`。公開 (`_`-接頭辞なし) /
  非公開 (`_`-接頭辞) のいずれも収集され、非公開のものは、公開
  FuncDef 本体や pack バインディングからの推移的到達性を経由して
  summary に昇格します。
- **相対インポート** — `>>> ./child.td => @(Sym1, Sym2)`。非相対パス
  (`>>> taida-lang/foo`、`>>> npm:*`、版指定インポート等) は拒否
  されます。
- **エクスポート宣言** — `<<< @(Sym1, Sym2)`。指定された場合、`<<<`
  句が正本となり、列挙されていないシンボルはユーザーインポートで
  名指せません。

現状拒否される構文 (現実のアドオンエコシステムで利用されていないため
情報提供にとどまります):

- ファサード内の `TypeDef` / `EnumDef` / `MoldDef` 文。
- `<<< <path>` 形式の再エクスポート。

これらに依存しているファサードがある場合、`taida build native` が
拒否対象の構文を指摘するコンパイルエラーを出します。暫定回避策は、
ローダーが理解する純 Taida 表現で欠落している構文をラップする
ファサード FuncDef を公開することです。

公開とリリースのワークフローは **タグ push のみ**: `taida ingot publish`
は git タグを push して即終了し、アドオンリポジトリ側の CI
(`.github/workflows/release.yml`、`taida init --target rust-addon` で
雛形が生成されます) が `github-actions[bot]` としてリリースをビルド
して公開します。対称な 4 ジョブパイプラインは
[§3 リリースワークフロー](#3-リリースワークフロー) を、既存アドオンの
追従手順は [§8 旧来アドオンの移行](#8-旧来アドオンの移行) を
参照してください。

---

## 0. `taida init --target rust-addon` で始める

ゼロから公開可能なアドオンへ最短で到達するには、組み込みの雛形を
使います。`taida init --target rust-addon` は、Rust クレート、ファサード、
マニフェスト、リリースワークフローまでを 1 ステップで書き出します。

```bash
$ taida init --target rust-addon my-addon
Initialized Taida project 'my-addon' (rust-addon) in my-addon
  packages.tdm
  Cargo.toml
  src/lib.rs
  native/addon.toml
  taida/my-addon.td
  .gitignore
  README.md
  .github/workflows/release.yml
```

生成される内容:

- **`packages.tdm`** — `<<<@a` プレースホルダー識別子付き。初回公開
  前に `<<<@a owner/my-addon @(MyExport, ...)` の qualified 形式に
  置き換えてください。bare な識別子のままだと `taida ingot publish` は
  拒否します。
- **`Cargo.toml`** — `crate-type = ["rlib", "cdylib"]` と
  `taida-addon = "2.0"` (ABI v1 用作者クレート) を設定済み。
- **`src/lib.rs`** — `declare_addon!` で `taida_addon_get_v1` から
  サンプル `echo` 関数をエクスポートする最小エントリポイント。
- **`native/addon.toml`** — `abi = 1`、`package` / `url` に `OWNER/...`
  プレースホルダー、空の `[library.prebuild.targets]` テーブル。CI が
  リリース時に `addon.lock.toml` 経由で SHA-256 ターゲットを補完します
  (§3 と §5 を参照)。
- **`taida/<name>.td`** — Taida 側のファサード。このパッケージから
  のインポートは、このファイルがエクスポートしたシンボル群に解決
  されます。
- **`.github/workflows/release.yml`** — Taida 本体のリリースワーク
  フローと対称なテンプレート。詳細は §3 を参照してください。

次にやるべきこと:

1. `native/addon.toml` 内の `OWNER` を GitHub の組織またはユーザー名
   (2 箇所) に置き換える。
2. `packages.tdm` の `<<<@a` プレースホルダーを qualified 形式に
   差し替え、エクスポートを宣言する。
3. `cargo build --release` で cdylib がビルドできることを確認する。
4. (任意) `native/addon.toml` の `prebuild.url` を相対 `file://target/release/lib<name>.so`
   に向けて、自分のビルド成果物に対するローカル `taida ingot install`
   を試す — §6 を参照。
5. 初回リリースの準備が整ったら、リポジトリを GitHub に push し、
   `taida ingot publish --dry-run` でバージョン繰り上げのプレビューを
   行う。`--dry-run` なしの `taida ingot publish` がタグを作成・push
   し、残りは CI が処理します (§3, §4)。

公式同梱アドオンの `taida-lang/terminal` も同じ雛形で構築されており、
その `.github/workflows/release.yml` は本リポジトリのテンプレートに
2 つのプレースホルダーを当てたものです。Taida 本体側のテストが雛形と
terminal の運用設定の対称性を継続的に検証しているため、雛形が drift
した場合は CI が落ちて検知されます。

---

## 1. ディレクトリ配置

最小構成のアドオンクレートは、対応するパッケージと並べて (あるいは内側に)
置きます。

```
my-addon/
  packages.tdm                  # Taida パッケージマニフェスト
  Cargo.toml                    # cdylib クレート
  src/lib.rs                    # アドオンエントリポイント
  native/
    addon.toml                  # インストール時マニフェスト
```

`Cargo.toml` ではクレートを `cdylib` として宣言してください。

```toml
[lib]
crate-type = ["cdylib"]
```

そして ABI 型のためにインツリーの `taida-addon` クレートに依存します。

---

## 2. addon.toml マニフェスト

プレビルド配布を含まない最小構成のマニフェスト (利用者は `.so` を
自分で配置する) は次のとおりです。

```toml
abi = 1
entry = "taida_addon_get_v1"
package = "my-org/my-addon"
library = "my_addon"

[functions]
greet = 1
```

`taida ingot install` でフェッチ可能なプレビルドを配布するには
`[library.prebuild]` セクションを追加します。

```toml
[library.prebuild]
url = "https://github.com/my-org/my-addon/releases/download/v{version}/lib{name}-{target}.{ext}"

[library.prebuild.targets]
"x86_64-unknown-linux-gnu"  = "sha256:<小文字 16 進 64 文字>"
"aarch64-unknown-linux-gnu" = "sha256:..."
"x86_64-apple-darwin"       = "sha256:..."
"aarch64-apple-darwin"      = "sha256:..."
"x86_64-pc-windows-msvc"    = "sha256:..."
```

### URL テンプレート変数

| 変数 | 展開対象 |
|------|----------|
| `{version}` | `taida ingot install` が解決したバージョン |
| `{target}` | ホストのターゲットトリプル (例: `x86_64-unknown-linux-gnu`) |
| `{ext}` | プラットフォームの cdylib 拡張子 (`so` / `dylib` / `dll`) |
| `{name}` | `[library] name` の値 |

未知の変数、対応していない波括弧、`{{` / `}}` のエスケープはマニフェスト
パース時に拒否されます — タイプミスを許容する余地はありません。

### 対応ホストターゲット

| トリプル | 状態 |
|----------|------|
| `x86_64-unknown-linux-gnu` | ベースライン |
| `aarch64-unknown-linux-gnu` | ベースライン |
| `x86_64-apple-darwin` | ベースライン |
| `aarch64-apple-darwin` | ベースライン |
| `x86_64-pc-windows-msvc` | ベースライン |
| `x86_64-unknown-linux-musl` | 拡張 |
| `aarch64-unknown-linux-musl` | 拡張 |
| `i686-unknown-linux-gnu` | 拡張 |
| `riscv64gc-unknown-linux-gnu` | 拡張 |
| `x86_64-unknown-freebsd` | 拡張 |

すべてのターゲット向けにバイナリを配布する必要はありません — 検証済み
のものだけで構いません。未掲載のターゲット上の利用者には、インストール
時に `addon is not available for your platform` という決定的なエラーが
返り、マニフェストで宣言済みのターゲットが列挙されます。

### SHA-256 整合性

`targets` の値は常に小文字 `sha256:` + 16 進 64 文字です。正準形を
強制するために大文字 16 進は拒否されます。`taida ingot install` は
ダウンロード済みバイト列を SHA-256 で stream し、不一致があれば
構造化エラーで中止します — サイレントなフォールバックはありません。

### 予約: `[library.prebuild.signatures]`

将来の GPG / 分離署名検証のために `signatures` 副テーブルが予約されて
います。

```toml
[library.prebuild.signatures]
"x86_64-unknown-linux-gnu" = "gpg:<opaque-identifier>"
```

値は `gpg:` で始まり、空でない印字可能 ASCII を続けてください。
Taida は現状これらの値を **パースして保存するだけ** で検証は行い
ません。今書いておいても安全です — このセクションを理解しない古い
taida はマニフェストごと拒否します (これは意図的な前方互換挙動です。
詳細は [アドオンマニフェスト](../reference/addon_manifest.md) を参照
してください)。

---

## 3. リリースワークフロー

アドオンのリリースパイプラインは **4 ジョブの CI ワークフロー**
(`.github/workflows/release.yml`) で、Taida 本体の `release.yml` と
構造的に対称です。`taida init --target rust-addon` で新規アドオン
クレートを作ると、このワークフローが自動で配置されます。古いレイアウト
のアドオンは移行が必要になる場合があります
([§8 旧来アドオンの移行](#8-旧来アドオンの移行) を参照)。

テンプレートは `taida init --target rust-addon` の雛形生成時に
次の 2 つのプレースホルダーが置換された形で書き出されます。

| プレースホルダー | 意味 |
|------------------|------|
| `{{LIBRARY_STEM}}` | cdylib のファイル名ステム (例: `taida_lang_terminal`)。`lib` 接頭辞・拡張子は含まない |
| `{{CRATE_DIR}}` | リポジトリルートから `Cargo.toml` までの相対パス (通常は `.`) |

### トリガー

ワークフローは次の 2 つのイベントで発火します。

- Taida のバージョン正規表現
  `^[a-z]\.[0-9]+(\.[a-z0-9][a-z0-9-]*)?$` に一致するタグへの `push`
  (bare — `@` 接頭辞なし)。例: `a.1`, `b.3.rc`, `aa.7.beta`。semver
  形式のタグ (`v1.2.3`) は意図的に無視されます。
- `tag` 入力付きの `workflow_dispatch`。既に push 済みのタグに対して
  手動で再実行するための入り口です。

### ジョブ

| ジョブ | 役割 |
|--------|------|
| `prepare` | タグ正規表現を検証し、ref を解決して `release_tag` / `release_ref` を出力 |
| `gate` | `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test --all` |
| `build` | 5 プラットフォームマトリクスで `cdylib` をビルドし、SHA-256 を計算してアーティファクトをアップロード |
| `publish` | マトリクスアーティファクトをダウンロードし、`addon.lock.toml` / `prebuild-targets.toml.txt` / `SHA256SUMS` を生成して `gh release create` を実行 |

5 プラットフォームマトリクスは次のとおりです。

| ランナー | ターゲットトリプル | `cross` 使用 |
|----------|--------------------|-------------|
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` | しない |
| `ubuntu-latest` | `aarch64-unknown-linux-gnu` | する |
| `macos-15-intel` | `x86_64-apple-darwin` | しない |
| `macos-14` | `aarch64-apple-darwin` | しない |
| `windows-latest` | `x86_64-pc-windows-msvc` | しない |

`publish` ジョブは `GH_TOKEN: ${{ github.token }}` で認証されるため、
**リリース作成者は常に `github-actions[bot]`** であり、`taida ingot publish`
を実行した人物ではありません。これはアドオンリリースパイプラインの
譲れない契約です。

### リリース成果物

`publish` ジョブが成功すると、GitHub リリースに 8 つのアセットが付随
します。

- 5 個の `lib<LIBRARY_STEM>-<triple>.<so|dylib|dll>` (マトリクス
  ビルド成果物)
- `addon.lock.toml` — CI が生成するロックファイル。各 cdylib の
  SHA-256 を列挙し、`taida ingot install` が正本として参照します。
- `prebuild-targets.toml.txt` — 必要に応じて
  `[library.prebuild.targets]` に貼り付けられる TOML 断片。ただし
  現行パイプラインではロックファイル側が主な情報源です。
- `SHA256SUMS` — 全アセットの SHA-256 を人間が確認するためのフラット
  なテキスト一覧。

### 参照実装

`taida-lang/terminal` がこのパイプラインの正準的な参照例です。本
パイプラインを通じて公開されており、CI 実行は同アドオンの GitHub
Actions タブから追えます。ワークフローファイル (テンプレートで
`LIBRARY_STEM=taida_lang_terminal` を当てたもの) もリリースアセット
構造も、信頼できる現物として参照できます。

---

## 4. `taida ingot publish` で新バージョンを公開する

アドオンクレートのルートで、`packages.tdm` の識別子を
`<<<@<version> <owner>/<name>` に設定したら、タグ付きリリースは 2
ステップで完了します。

```bash
# 1. プレビュー: どのバージョンが公開されるか
$ taida ingot publish --dry-run
Publish plan for my-org/my-addon:
  Last release tag: a.3
  API diff: added 2
  Next version: a.4
  Tag to push: a.4
  Remote: origin
  Dry-run: no git changes performed.

# 2. 実行: タグを push して即終了
$ taida ingot publish
Created tag 'a.4' and pushed to origin.
CI will build and publish the release.
```

`taida ingot publish` は CI の完了を **待ちません**。GitHub Actions の
タブを開いて 4 ジョブの進行を確認してください (ベースラインの 5
プラットフォームマトリクスで概ね 90 秒程度)。

### 自動バージョン繰り上げ

`taida ingot publish` は前回リリースタグと HEAD の間で `taida/*.td` の
エクスポートシンボル集合を比較します。繰り上げ表は
[CLI リファレンスの ingot publish 節](../reference/cli.md#ingot-publish)
を参照してください。1 行要約は次のとおりです。

- シンボル削除 / 改名 → 世代繰り上げ (`a.3` → `b.1`)
- シンボル追加 / 内部変更のみ → 番号繰り上げ (`a.3` → `a.4`)
- 直前のタグが存在しない → `a.1` (初回リリース)

### エスケープハッチ

| フラグ | 用途 |
|--------|------|
| `--force-version a.5` | 自動検出されたバージョンを上書き (API diff をスキップ) |
| `--label rc` | プレリリースラベルを付与 (`a.4` → `a.4.rc`) |
| `--retag` | 既に push 済みのタグを強制置換 (API diff をスキップ) |

`--force-version` と `--retag` は意図的に API diff スナップショットを
回避します。古いパッケージ (現行パーサーが拒否する構文 — 例:
`[E1616]` の discard binding — を含む可能性のあるもの) を、過去タグ
の `taida/*.td` を Taida パーサーに通さずに再タグ付けできるようにする
ためです。

---

## 5. `taida ingot install` がプレビルドを取得する流れ

パッケージが `[library.prebuild]` セクション付きの `native/addon.toml`
を持っているとき、`taida ingot install` は次の順に処理を進めます。

1. ホストのターゲットトリプルを検出する。
2. ホストトリプルを `[library.prebuild.targets]` 内で検索する。未知の
   ホストは決定的なエラーになり、マニフェストが宣言した全ターゲットが
   列挙される。
3. **SHA 取得元の選択**: タグ時点の `addon.toml` がプレースホルダー
   SHA (`sha256:` + 0 が 64 文字) を含む場合、リゾルバはリリース
   アセットの `addon.lock.toml` を正本としてフォールバックする。
   初回リリース作成者が `[library.prebuild.targets]` をプレース
   ホルダーのままにし、正準ロックファイルの公開を CI に委ねているケース
   の標準動作。
4. URL テンプレートの `{version}` / `{target}` / `{ext}` / `{name}` を
   展開する。
5. バイナリを HTTPS でダウンロード (最大 10 リダイレクト)、もしくは
   `file://` URL を読み込み (相対パスのみ。セキュリティモデルは
   [アドオンマニフェスト](../reference/addon_manifest.md) を参照)。
6. バイト列を SHA-256 で stream して照合し、不一致を拒否。
7. 検証済みバイナリを
   `~/.taida/addon-cache/<org>/<name>/<version>/<target>/lib<name>.<ext>`
   にキャッシュし、`.taida/deps/<pkg>/native/lib<name>.<ext>` に作業用
   コピーを配置。
8. ターゲットとハッシュの組を `taida.lock` に `[[package.addon]]` 副
   テーブルとして書き込み、再現可能なインストールが再ダウンロード
   なしで連鎖を検証できるようにします。

ダウンロードが概ね 256 KiB を超えた段階で、標準エラー出力にバイト数
ベースの進捗表示が出ます。再ダウンロードを強制する場合は
`taida ingot install --force-refresh`、キャッシュを丸ごと整理する場合は
`taida ingot cache clean --addons` を使ってください。

---

## 6. `file://` を使ったローカルテスト

開発中は GitHub に公開する必要はありません。URL テンプレートを
相対 `file://` パスに向けます。

```toml
[library.prebuild]
url = "file://target/release/libmy_addon.so"

[library.prebuild.targets]
"x86_64-unknown-linux-gnu" = "sha256:<ビルドごとに再計算>"
```

制約:

- `file://` 配下は **相対** パスのみ受け付けます。絶対パスと `..`
  コンポーネントは、いかなるファイルシステムアクセスよりも前に拒否
  されます。
- パスは `packages.tdm` を含むプロジェクトルートからの相対で解決
  されます。
- 整合性チェックは引き続き走るため、再ビルドのたびに SHA-256 を更新
  してください。

Taida 本体側でも、サンプルアドオンクレートをこの方法でエンドツーエンド
テストしています。

---

## 7. リリース前チェックリスト

- [ ] `packages.tdm` で qualified 識別子
      (`<<<@<version> <owner>/<name>`) を宣言している — `<owner>/<name>`
      を伴わない bare な `<<<@<version>` は `taida ingot publish` が
      拒否する
- [ ] `.github/workflows/release.yml` が標準テンプレート (4 ジョブ、
      5 プラットフォームマトリクス、リリース作成者 `github-actions[bot]`)
      である。`prebuild.yml` を使っている旧アドオンは
      [§8 旧来アドオンの移行](#8-旧来アドオンの移行) で移行する
- [ ] push 予定のタグが origin にまだ存在しない (あるいは `--retag` を
      意図的に渡している)
- [ ] `cargo build --release` がローカルで通る (CI マトリクスがクロス
      ターゲット問題を捕まえますが、ローカル x86_64 の失敗は早く落とす)
- [ ] 開発中にローカル `file://` URL に対して `taida ingot install` が
      エンドツーエンドで完了する
- [ ] `[functions]` 表が `declare_addon!` でエクスポートしたすべての
      シンボルを列挙している
- [ ] README に **最小対応 taida バージョン** を記載している (古い
      taida は未知のマニフェストキーを設計どおり拒否するため。
      詳細は [アドオンマニフェスト](../reference/addon_manifest.md))

---

## 8. 旧来アドオンの移行

`taida publish --target rust-addon` と `prebuild.yml` ワークフローを
使い、リリース作成者が CLI 実行者になっていた旧来のアドオンは、現行
パブリッシュパイプラインで動かすために機械的な調整が必要です。

### Step 1 — `packages.tdm` に識別子を付与する

owner / name を伴わない bare 形式の `packages.tdm`:

```taida
// 旧形式
<<<@a.1
>>> ./main.td => @(...)
```

を qualified 形式に変更します。

```taida
// 現行形式
>>> ./main.td
<<<@a.1 <owner>/<name> @(...)
```

`taida ingot publish` は bare な `<<<@<version>` (識別子なし) を
拒否します。リゾルバが `owner/name` 修飾を伴わずにフェッチ URL を
導出できないためです。

### Step 2 — `prebuild.yml` を現行 `release.yml` テンプレートに置き換える

旧来の `prebuild.yml` は 2 ジョブ (Build + Release-attach) しか持たず、
`gh release create` を CLI 実行者として既に走らせている前提でした。
現行パイプラインではリリース作成は CI 側が握ります。

選択肢 A — 雛形再生成: 別ディレクトリで `taida init --target rust-addon
my-addon` を実行し、生成された `.github/workflows/release.yml` をコピー
してファイル先頭の `LIBRARY_STEM` と `CRATE_DIR` の env 値を既存
プロジェクトに合わせます。これで `prebuild.yml` を置き換え、旧ファイル
を削除します。

選択肢 B — 既存アドオンを参考にコピー: 公式同梱の `taida-lang/terminal`
の `.github/workflows/release.yml` をテンプレートとしてコピーし、
ファイル先頭の `LIBRARY_STEM` (cdylib ステム、`lib` 接頭辞・拡張子なし)
と `CRATE_DIR` (リポジトリルートから `Cargo.toml` までの相対パス、通常
`.`) を自分のプロジェクトに合わせて書き換えます。

いずれの場合も、次の内容の PR を用意してください。

- `.github/workflows/prebuild.yml` の削除
- `.github/workflows/release.yml` の追加
- 既存テストが検証しているタグ命名スキーム (`a.1`, `b.1.rc` 等) の
  維持

### Step 3 — プレースホルダー `addon.toml` + CI 生成 `addon.lock.toml` を受け入れる

現行テンプレートは `addon.lock.toml` を SHA の正本としてリリース
アセットに添付します。追跡対象の `native/addon.toml` では次のいずれかを
選んでください。

- (a) `[library.prebuild.targets]` を `main` 上ではプレースホルダー値
  (`sha256:` + 0 が 64 文字) のまま残す。
- (b) セクションごと削除する。

どちらの経路もリゾルバが対応しています — 選択肢 (b) はよりクリーン
ですが、毎リリースが `addon.lock.toml` を出すことが前提です。選択肢
(a) は将来のリリースがロックファイルを欠落させた場合の保険にもなり
ます。

`taida ingot install` はプレースホルダー SHA を自動検出し、リリース
アセットの `addon.lock.toml` にフォールバックします。完全な判定
マトリクスは [アドオンマニフェスト](../reference/addon_manifest.md) を
参照してください。

### Step 4 — スクリプトから廃止済み CLI オプションを除去する

`Makefile` / シェルエイリアス / CI スクリプトが旧パブリッシュ surface
を呼んでいる場合、`taida ingot publish` に切り替え、廃止済みオプション
を渡さないようにしてください。

| 旧形式 | 現行の置き換え |
|--------|----------------|
| `taida publish --target rust-addon` | `taida ingot publish` (target は暗黙) |
| `taida publish --dry-run=plan` | `taida ingot publish --dry-run` |
| `taida publish --dry-run=build` | 廃止 — ローカルビルドは CI のみで実施 |
| `TAIDA_PUBLISH_SKIP_RELEASE=1` | 廃止 — CLI はもはやリリースを作成しない |

### Step 5 — (任意) 初回リリースを再タグ付けする

旧 CLI で push されたアドオンの既存 `a.1` タグ (= リリース作成者が
人間で、`github-actions[bot]` ではない) は、同じタグに対して現行
パイプラインを再実行できます。

```bash
taida ingot publish --force-version a.1 --retag
```

これはタグを origin 上で強制置換し、新しい `release.yml` を発火させ、
作成者 `github-actions[bot]` でリリースを作り直し、8 アセット (5 個の
cdylib + `addon.lock.toml` + `prebuild-targets.toml.txt` + `SHA256SUMS`)
を再付与します。

`--force-version` と `--retag` は同時に API diff スナップショットを
スキップするため、旧タグの `taida/*.td` が含む古い構文 (現行パーサー
が拒否するもの) によって Taida パーサーが再タグを阻むことはありません。
