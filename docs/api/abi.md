# `taida-lang/abi` API リファレンス

`taida-lang/abi` は、外部ホストが Taida 関数をリクエストハンドラとして
呼び出すためのコア同梱パッケージです。HTTP 系ホストやローカル adapter
は、`WebRequest` を渡し、`WebResponse` を受け取ります。

```taida
>>> taida-lang/abi => @(WebRequest, WebResponse, text, json, bytes, status, header)
```

---

## 1. 型

### 1.1 `WebRequest`

```taida
WebRequest = @(
  method: Str,
  path: Str,
  query: HashMap[Str, Str],
  headers: HashMap[Str, Str],
  body: Bytes
)
```

| Field | Type | Description |
|-------|------|-------------|
| `method` | `Str` | HTTP method。例: `"GET"` / `"POST"`。 |
| `path` | `Str` | path 部分。query string は含めません。 |
| `query` | `HashMap[Str, Str]` | query parameter。 |
| `headers` | `HashMap[Str, Str]` | header 名から値への map。 |
| `body` | `Bytes` | request body の生 bytes。 |

### 1.2 `WebResponse`

```taida
WebResponse = @(
  status: Int,
  headers: HashMap[Str, Str],
  body: Bytes
)
```

| Field | Type | Description |
|-------|------|-------------|
| `status` | `Int` | HTTP status code。未指定時の helper 既定値は `200`。 |
| `headers` | `HashMap[Str, Str]` | response headers。 |
| `body` | `Bytes` | response body の生 bytes。 |

---

## 2. 関数

| 関数 | シグネチャ | 概要 |
|------|------------|------|
| `text` | `text body: Str => :WebResponse` | UTF-8 text response。`content-type` は `text/plain; charset=utf-8`。 |
| `json` | `json value => :WebResponse` | `jsonEncode(value)` 相当の JSON response。`content-type` は `application/json`。 |
| `bytes` | `bytes body: Bytes => :WebResponse` | bytes response。`content-type` は `application/octet-stream`。 |
| `status` | `status code: Int  response: WebResponse => :WebResponse` | status code を差し替えた response を返す。範囲外は `100`〜`599` に丸めます。 |
| `header` | `header name: Str  value: Str  response: WebResponse => :WebResponse` | header を追加または上書きした response を返す。無効な header 名や改行を含む値は 500 response になります。 |

`status` / `header` は入力 response を変更せず、新しい `WebResponse` を返します。

**Example**:

```taida
>>> taida-lang/abi => @(WebRequest, WebResponse, text, status, header)

handle req: WebRequest =
  header("x-taida", "ok", status(201, text("created: " + req.path)))
  => :WebResponse
```

---

## 3. Handler Mode

`taida build native` と各 WASM target は `--handler <SYMBOL>` を受け付けます。
handler 関数は、必ず 1 つの `WebRequest` を受け取り、`WebResponse` を返します。

```bash
taida build native --handler handle app.td -o app-handler
taida build wasm-edge --handler handle app.td -o app.wasm
```

```taida
>>> taida-lang/abi => @(WebRequest, WebResponse, text)

handle req: WebRequest =
  text(req.method + " " + req.path)
  => :WebResponse
```

handler mode は次の条件を満たさない場合、コンパイル時に拒否されます。

| 条件 | 失敗時 |
|------|--------|
| `taida-lang/abi` を import している | `E1961` |
| handler symbol が entry source 内の関数として存在する | `E1961` |
| handler が引数を 1 つだけ持つ | `E1961` |
| 引数型が `WebRequest` | `E1961` |
| 戻り型が `WebResponse` | `E1961` |

Native handler binary は、標準入力から request JSON を 1 件読み、標準出力へ
response JSON を 1 件出力します。WASM handler output は、host adapter が
linear memory の ABI export を呼び出して同じ JSON 形へ変換します。

Request JSON:

```json
{
  "method": "POST",
  "path": "/items",
  "query": { "q": "taida" },
  "headers": { "content-type": "text/plain" },
  "bodyBase64": "aGVsbG8="
}
```

Response JSON:

```json
{
  "status": 200,
  "headers": { "content-type": "text/plain; charset=utf-8" },
  "bodyBase64": "T0s="
}
```

`bodyBase64` は `Bytes` を JSON 上で安全に運ぶための wire 表現です。Taida
コード内では `body` フィールドは `Bytes` として扱います。

Native / WASM の bridge は、欠落または解釈できない request フィールドに
既定値を入れます。既定値は `method = "GET"`、`path = "/"`、空の `query`、
空の `headers`、空の `body` です。

Native / WASM handler bridge は 1 request の wire JSON を最大 16MiB として
扱います。上限を超える request は 413 response に変換されます。

WASM の生成済み Edge glue は fetch ごとに新しい WebAssembly instance を作り、
request 完了時に破棄します。独自 host で同じ instance を複数 request に再利用
する場合も、`taida_abi_web_free(handle)` を呼ぶとその request の allocator 領域は
巻き戻されます。host は `out_ptr` / `out_len` で response JSON を読み終えてから
必ず `free(handle)` を呼び、`free` 後の pointer を保持しないでください。

WASM handler の `req.body` は `Bytes` surface として扱えます。`bytes(req.body)`
による response body 転送、`req.body.length()`、`req.body.get(i)` は Native /
WASM handler bridge の portable な契約です。

---

## 4. バックエンド対応

| バックエンド | 対応範囲 |
|--------------|----------|
| インタプリタ | 型・helper 評価に対応。handler binary は生成しません。 |
| ネイティブ | `--handler` で request/response JSON bridge を持つ binary を生成。 |
| WASM (`wasm-min` / `wasm-wasi` / `wasm-edge` / `wasm-full`) | `--handler` と共有 WebRequest/WebResponse ABI に対応。 |
| 旧 JS ターゲット | handler mode 非対応。 |
