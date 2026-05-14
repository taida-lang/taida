# プリリュード関数リファレンス

プリリュードはインポート不要で常に利用可能な関数群です。
入出力、時間取得、JSON シリアライズ、デバッグ出力、整数列生成などの
基本ユーティリティを提供します。

すべての関数のシグネチャは Taida 流に「全ての引数にデフォルト値を持つ」
前提で書かれています。引数を省略した場合は、その型のデフォルト値
(`Int` なら `0`、`Str` なら `""`) で呼び出された扱いになります。例外的に
省略時の挙動が型デフォルトと異なる関数は本書内で明示します。

プリリュード「型コンストラクタ」(`Lax` / `Result` / `Gorillax` /
`hashMap` / `setOf` 等) は本書ではなく
[`docs/reference/standard_library.md`](../reference/standard_library.md)
を参照してください。

---

## 1. 入出力

### 1.1 `stdout`

```
stdout(value: Str) -> Int
```

標準出力に `value` を書き出し、書き込んだ UTF-8 バイト数 (末尾の自動
改行を除く) を `Int` で返します。

- 各バックエンドで暗黙の改行 `\n` が末尾に付加されます。
- パイプ下流が閉じている場合 (`SIGPIPE` 相当) はサイレントに `0` を返し、
  プロセスは `exit 0` で終了します。
- `value` は `Str` 以外の型でも自動 stringify されます (`Int` / `Float` /
  `Bool` / ぶちパック / リスト等)。

```taida
bytes <= stdout("Hello")        // bytes = 5
stdout(42)                      // 出力: "42"
```

### 1.2 `stderr`

```
stderr(value: Str) -> Int
```

標準エラー出力に書き出します。それ以外の挙動は `stdout` と同じです。

`stdout` と異なり、REPL / Rust 側 in-process テスト API でも呼び出し
ごとに即時 flush されます (バッファリングしません)。

### 1.3 `stdin`

```
stdin(prompt: Str) -> Str
```

標準入力から 1 行を読み取って `Str` で返します。改行コードは自動で
除去されます。

- `prompt` を渡すと、読み取り前に標準出力へ表示します (デフォルト値
  `""` の場合は何も表示しません)。
- EOF / IO エラー時はサイレントに空文字列 `""` を返します。エラーを
  検知したい場合は `stdinLine` を使ってください。
- ASCII 入力やパイプ用途を想定しています。マルチバイト編集 (Backspace
  で UTF-8 コードポイント単位を扱う等) は `stdinLine` 側に対応がありま
  す。

```taida
name <= stdin("お名前: ")
stdout("こんにちは、" + name)
```

### 1.4 `stdinLine`

```
stdinLine(prompt: Str) -> Async[Lax[Str]]
```

UTF-8-aware なライン編集対応の標準入力読み取りです。Backspace で
マルチバイト 1 文字単位の削除、Ctrl-C / Ctrl-D による中断検知をサポート
します。

- 戻り値は `Async[Lax[Str]]`。`]=>` で待ち、さらに `]=>` で `Lax[Str]`
  を展開します。
- EOF / Ctrl-C / Ctrl-D / IO エラー時は `Lax.failure` を返します。
- `prompt` の扱いは `stdin` と同じです (デフォルト値 `""` で何も表示
  しない)。

```taida
stdinLine("名前: ") ]=> name_lax
name <= name_lax.getOrDefault("ゲスト")
```

---

## 2. 時間

### 2.1 `nowMs`

```
nowMs() -> Int
```

Unix epoch (1970-01-01T00:00:00Z) からの経過ミリ秒数を `Int` で返します。

- **ウォールクロックであり、単調時計ではありません**。NTP 補正や手動
  時刻変更でジャンプ・巻き戻しが発生する可能性があります。
- 経過時間の厳密測定 (タイムアウト・レート制御・パフォーマンス計測) で
  使う場合は許容誤差を併用してください。
- 4 バックエンドすべてで同じウォールクロック契約に従います。解像度は
  OS / ホスト依存です。

```taida
start <= nowMs()
sleep(10) ]=> _
end <= nowMs()
stdout((end - start).toString())   // 例: "10"
```

### 2.2 `sleep`

```
sleep(ms: Int) -> Async[Unit]
```

`ms` ミリ秒の待機を行う非同期処理を返します。`]=>` で展開すると待機が
完了した時点で `Unit` が得られます。

- `ms <= 0` の場合は即座に完了します (待機なし)。
- バックエンドごとに OS / ホストランタイム提供のスリープに委譲します。

```taida
sleep(100) ]=> _
stdout("100ms 経過")
```

---

## 3. JSON シリアライズ

### 3.1 `jsonEncode`

```
jsonEncode(value: Any) -> Str
```

ぶちパック / リスト / プリミティブ / `Lax` などを JSON 文字列に変換
します。出力は 1 行で最小化されており、空白を含みません。

- ぶちパックのキーは Taida 識別子のまま JSON キーになります。
- 内部フィールド名 (`has_value` / `__value` 等) はそのまま出力されます。
- `Lax` / `Gorillax` / `Result` は内部表現がそのまま JSON 化されます。

```taida
pilot <= @(name <= "Misato", age <= 29)
stdout(jsonEncode(pilot))
// 出力: {"name":"Misato","age":29}
```

### 3.2 `jsonPretty`

```
jsonPretty(value: Any) -> Str
```

`jsonEncode` と同じ入力を整形 JSON 文字列に変換します。インデントは
2 スペース固定です。

```taida
stdout(jsonPretty(@(name <= "Misato", age <= 29)))
// 出力:
// {
//   "name": "Misato",
//   "age": 29
// }
```

JSON のパース (外部データ → Taida 値) は `JSON[raw, Schema]()` モールド
を使います。詳細は [`docs/guide/03_json.md`](../guide/03_json.md) を
参照してください。

---

## 4. デバッグ / 内省

### 4.1 `debug`

```
debug(value: Any) -> Any
debug(value: Any, label: Str) -> Any
```

`value` を標準出力に表示し、その `value` をそのまま返します。
パイプラインの途中に挿入できる「副作用付き恒等関数」として使えます。

- `label` を渡すと `"label: <value>"` の形式で表示されます (デフォルト値
  `""` の場合はラベル無し)。
- 戻り値はそのまま `value` なので、`x => debug => y` の形でパイプライン
  をそのまま継続できます。
- 4 バックエンドすべてで標準出力に書きます。

```taida
scores <= @[95, 82, 78, 91]
scores
  => Filter[_, _ x = x > 80]()
  => debug                      // フィルタ後の値が表示される
  => Map[_, _ x = x * 2]()
  => result
```

### 4.2 `typeof`

```
typeof(value: Any) -> Str
```

`value` の宣言型 (静的型) の名前を `Str` で返します。コンパイル時情報を
ランタイムから取り出すためのヘルパーです。

```taida
typeof(42)              // "Int"
typeof("hello")         // "Str"
typeof(@[1, 2, 3])      // "@[Int]"
```

値の type identity (クラスライク継承位置の `__type` 相当) を取り出した
い場合は、関数ではなくモールド `TypeName[value]()` を使います。詳細は
[`docs/reference/class_like_types.md`](../reference/class_like_types.md)
を参照してください。

---

## 5. 整数列生成

### 5.1 `range`

```
range(start: Int, end: Int) -> @[Int]
range(start: Int, end: Int, step: Int) -> @[Int]
```

`start` から `end - 1` までの整数リストを生成します。

- `step` を省略した場合 (デフォルト値 `1`) は 1 ずつ増加します。
- `start >= end` の場合は空リスト `@[]` を返します。
- `step` を `0` で呼び出すと空リスト `@[]` を返します (無限ループ防止)。
- 負の `step` を指定すると、`start` から `end + 1` まで降順で生成
  します。

```taida
range(0, 5)            // @[0, 1, 2, 3, 4]
range(0, 10, 2)        // @[0, 2, 4, 6, 8]
range(5, 0)            // @[]
```

---

## 6. プロセス制御

### 6.1 `exit`

```
exit(code: Int) -> Unit
```

プロセスを `code` で終了します。後続の文は実行されません。

- `code` のデフォルト値 `0` は正常終了です。慣例的に異常終了は `1` 以上
  の値を使います。
- `]=>` の途中で呼ばれても、現在の `Async` を完了させずに即座に終了
  します。

```taida
| has_error
  |> stderr("致命的エラー") => exit(1)
  | _ |> stdout("正常終了")
```

---

## 7. バックエンド対応

プリリュード関数はすべて 4 バックエンド (インタプリタ / ネイティブ /
JS / WASM 全プロファイル) で同一の挙動を返します。例外は次のとおりです。

| 関数 | 例外バックエンド | 補足 |
|------|------------------|------|
| `stdinLine` | `wasm-min` / `wasm-edge` で利用不可 | WASI 入力の TTY 抽象が必要 |

詳細なバックエンドごとの差異は
[`docs/reference/wasm_profiles.md`](../reference/wasm_profiles.md) と
[`docs/reference/memory_model.md`](../reference/memory_model.md) を参照
してください。
