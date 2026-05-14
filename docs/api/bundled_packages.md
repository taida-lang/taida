# 同梱パッケージリファレンス

このリファレンスは、Taida に標準で同梱されるコアパッケージの一覧と、
それぞれのインポート方法・対応バックエンド・公開 API への入口を
まとめます。

Taida のパッケージは大きく次の 3 層に分かれます。

1. **プリリュード** — `stdout` / `nowMs` / `jsonEncode` 等。インポート
   不要で常に利用可能。[標準ライブラリ](../reference/standard_library.md)
   を参照してください。
2. **コア同梱パッケージ** — `taida-lang/os` / `taida-lang/net` /
   `taida-lang/crypto` / `taida-lang/js` / `taida-lang/pool`。Taida
   バイナリに同梱されており、`taida ingot install` などのインストール
   は不要です。`>>> taida-lang/<pkg> => @(...)` で明示インポート、
   または import なしでの直接呼び出しに両対応します。
3. **公式アドオン** — `taida-lang/terminal` など。ネイティブ cdylib
   を `taida ingot install` で取得するインゴットとして配布されます。
   詳細は [アドオン作成ガイド](../guide/13_creating_addons.md) を参照
   してください。

本書はこのうち **2 のコア同梱パッケージ** を扱います。

---

## コア同梱パッケージ一覧

| パッケージ | 概要 | 詳細リファレンス | ガイド |
|------------|------|------------------|--------|
| [`taida-lang/os`](#taida-langos) | ファイル I/O、プロセス、環境変数、低水準ソケット、DNS | [`os.md`](os.md) | [`14_os_package.md`](../guide/14_os_package.md) |
| [`taida-lang/net`](#taida-langnet) | HTTP/1.1・H2・H3、WebSocket、SSE | [`net.md`](net.md) | [`15_net_package.md`](../guide/15_net_package.md) |
| [`taida-lang/crypto`](#taida-langcrypto) | 暗号系プリミティブ (`sha256`) | [`crypto.md`](crypto.md) | — |
| [`taida-lang/js`](#taida-langjs) | JS 相互運用 (モールテン境界) | [`js.md`](js.md) | — |
| [`taida-lang/pool`](#taida-langpool) | 接続プーリングの最小契約 | [`pool.md`](pool.md) | — |

すべてのコア同梱パッケージは、現行 Taida のバージョン記法
`@<世代>.<番号>[.<ラベル>]` で参照できます。版指定の書き方は
[命名規則](../reference/naming_conventions.md) の Versioning 節を参照
してください。

---

## taida-lang/os

`taida-lang/os` は OS 系の標準パッケージです。ファイル I/O、プロセス
管理、環境変数、低水準ソケット、DNS 解決などを提供します。

### インポート

```taida
// 明示インポート (推奨)
>>> taida-lang/os => @(readFile, writeFile, env, args)

// import なしで直接呼ぶことも可能
content <= readFile("config.toml")
```

### バックエンド対応

| バックエンド | 対応範囲 |
|--------------|----------|
| インタプリタ | 全 API |
| ネイティブ | 全 API |
| JS | 全 API (Node.js host を想定) |
| WASM (`wasm-min`) | 利用不可 |
| WASM (`wasm-wasi`) | WASI で文書化された部分集合 |
| WASM (`wasm-edge`) | エッジホストで文書化された部分集合 |
| WASM (`wasm-full`) | `wasm-wasi` と同じ部分集合 |

各 API のシグネチャ・戻り値・失敗条件は [`os.md`](os.md) を、
ナラティブ学習用には [`14_os_package.md`](../guide/14_os_package.md) を
参照してください。

---

## taida-lang/net

`taida-lang/net` はネットワーク標準パッケージです。HTTP サーバー /
クライアント、WebSocket、Server-Sent Events を扱います。低水準ソケット
や DNS は `taida-lang/os` 側に置かれています。

### インポート

```taida
>>> taida-lang/net => @(httpServe, readBody, startResponse, writeChunk, endResponse)
```

### 公開シンボル

`httpServe` / `httpParseRequestHead` / `httpEncodeResponse` /
`readBody` / `startResponse` / `writeChunk` / `endResponse` /
`sseEvent` / `readBodyChunk` / `readBodyAll` / `wsUpgrade` /
`wsSend` / `wsReceive` / `wsClose` / `wsCloseCode` /
`HttpProtocol` (Enum で `:H1` / `:H2` / `:H3` を提供)。

### バックエンド対応

| バックエンド | 対応範囲 |
|--------------|----------|
| インタプリタ | 全 API |
| ネイティブ | 全 API |
| JS | 全 API |
| WASM (`wasm-min`) | 利用不可 |
| WASM (`wasm-wasi`) | `httpServe` の plaintext HTTP/1.1 部分集合 |
| WASM (`wasm-edge`) | 利用不可 |
| WASM (`wasm-full`) | `wasm-wasi` と同じ部分集合 |

詳細なシグネチャ・zero-copy span pack・WebSocket / SSE 契約は
[`net.md`](net.md) を、ナラティブ学習用には
[`15_net_package.md`](../guide/15_net_package.md) を参照してください。

---

## taida-lang/crypto

暗号系プリミティブを提供します。現行公開 surface は SHA-256 ハッシュ
のみです。

```taida
>>> taida-lang/crypto => @(sha256)
```

詳細な API シグネチャ、バックエンド対応、将来拡張領域は
[`crypto.md`](crypto.md) を参照してください。

---

## taida-lang/js

JavaScript エコシステムとの相互運用 (interop) を提供します。`Cage`
モールドと組み合わせて、外部 JavaScript ライブラリのオブジェクト・
メソッド呼び出しを Taida 側から安全に行う descriptor 群を公開
します。

```taida
>>> taida-lang/js => @(JSGet, JSCall, JSNew, JSSet, JSBind, JSSpread)
```

`taida-lang/js` は **JS バックエンド専用** です。詳細は
[`js.md`](js.md) を参照してください。

---

## taida-lang/pool

リソースプーリングの最小契約を提供します。プールの生成・取得・返却・
破棄・状態観測を共通 API として公開し、HTTP クライアントコネクション
や DB ドライバ接続の上位ライブラリから利用されることを想定しています。

```taida
>>> taida-lang/pool => @(poolCreate, poolAcquire, poolRelease, poolClose, poolHealth)
```

各 API のシグネチャ、`config` フィールドの既定値、失敗条件は
[`pool.md`](pool.md) を参照してください。

---

## 公式アドオン (参考)

`taida-lang/terminal` のような公式アドオンは、本書のコア同梱
パッケージとは別カテゴリです。`taida ingot install` 経由で取得する
ネイティブ cdylib として配布され、`packages.tdm` に依存宣言を追加する
必要があります。詳細は次の各文書を参照してください。

- [`taida-lang/terminal` パッケージガイド](../guide/16_terminal_package.md)
- [アドオン作成ガイド](../guide/13_creating_addons.md)
- [アドオンマニフェストリファレンス](../reference/addon_manifest.md)
