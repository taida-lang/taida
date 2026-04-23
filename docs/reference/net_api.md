# `taida-lang/net` API リファレンス

> Core bundled package. `>>> taida-lang/net => @(...)` で import、または import 無しで直接呼び出し可能（両経路とも checker で同じ型 signature に pin されます）。

ガイドは存在しません（`docs/guide/` 未作成）。3-backend (Interpreter / JS / Native) parity を GA 条件として C26 で仕上げ中です — `.dev/C26_BLOCKERS.md::C26B-001〜C26B-006` を参照。

---

## 1. 設計方針

Taida の NET surface は **zero-copy span** を基本単位とします:

- `httpServe` handler / `httpParseRequestHead` が返す `req` pack の `method` / `path` / `query` / `headers[i].name` / `headers[i].value` / `body` は **`@(start: Int, len: Int)` の span pack** で、元の `req.raw: Bytes` に対する view です。
- 原本の `Bytes` を clone せず、必要になった時点で user が明示的に **span → Str** または **span-aware 比較** を呼ぶ形にしています。これは C26B-018 / C26B-024 の clone-heavy 抑制方針 (`src/interpreter/value.rs` の Arc + try_unwrap COW 共通 abstraction) と一致する設計です。
- span pack を受け取る公開 mold 群は `§ 4 span-aware 比較 mold` を参照。
- 「`req.method` を自動で `Str` に昇格する」設計 (Option A) は `tests/parity.rs` 既存 assertion (`body <= req.method` 等) を破壊するため D27 送り (`.dev/D27_BLOCKERS.md`) に pin 済みです。gen-C では Option B+ (span 保持 + span-aware 公開 mold 追加) で ergonomics を解決します。

---

## 2. Server

### 2.1 `httpServe`

```
httpServe(port: Int, handler: Fn, ?opts: BuchiPack) -> Gorillax[@(closed: Bool)]
```

- `port` — bind port。`0` を渡すと OS 割り当て (port は `opts` で返さない、観測には `getsockname` 相当の mold を別途使う想定。C26B-003 で port-bind race 根治の一環として確定予定)。
- `handler` — 下記 2.2 / 2.3 のいずれかの arity を持つ関数値。
- `opts` (optional) — TLS 設定 `@(cert: Str, key: Str, protocol: Str)` 等。`protocol <= "h2"` を指定すると HTTP/2 over TLS (C26B-001)。TLS 細目は C26B-002 で pin 中。

### 2.2 1-arg handler (response-return form)

```
handler req: BuchiPack = ... => :Str
```

戻り値は HTTP wire 文字列 (`"HTTP/1.1 200 OK\r\n..."`) または `Bytes`。Interpreter / Native で **`src/interpreter/net_eval/h1.rs:1156`** の経路を通ります。

### 2.3 2-arg handler (streaming form)

```
handler req: BuchiPack writer: BuchiPack = ... => :Unit
```

`writer` pack は下記 `writer.write(bytes)` / `writer.end()` などを持つ streaming API。Interpreter / Native で **`src/interpreter/net_eval/h1.rs:840`** の経路を通ります。

> **Known issue (C26B-023, Must Fix)**: 2-arg form で handler 内から `req.body` を直接参照すると span の `len` が 0 になる edge case あり。body を読む場合は `readBody(req)` / `readBodyChunk(req)` / `readBodyAll(req)` を使うこと。現状は diagnostic 無しで silent breakage になるため C26 で runtime warning 追加予定 (`diagnostic_codes.md` の新 E1xxx を C26B-023 FIXED 時に pin)。

---

## 3. `req` pack shape

`httpServe` handler / `httpParseRequestHead` の返り値 pack は 2 arity で **フィールドの有無が異なります**:

### 3.1 1-arg handler req shape

| Field | Type | 意味 |
|-------|------|------|
| `raw` | `Bytes` | 生 request wire bytes (head + body 全体) |
| `method` | `@(start: Int, len: Int)` | method の span (view over `raw`) |
| `path` | `@(start: Int, len: Int)` | path の span |
| `query` | `@(start: Int, len: Int)` | query の span (query 無しの場合 `len = 0`) |
| `version` | `@(major: Int, minor: Int)` | HTTP version (`major=1 minor=1` / `major=2 minor=0` / ...) |
| `headers` | `@[@(name: span, value: span)]` | header リスト。`name` / `value` 双方が span pack |
| `body` | `@(start: Int, len: Int)` | body の span (`start = bodyOffset`, `len = contentLength` 想定) |
| `bodyOffset` | `Int` | head 終端 offset (= body start) |
| `contentLength` | `Int` | Content-Length header 値 (明示されない場合 `0` または chunked の累積) |
| `remoteHost` | `Str` | peer address の IP string (`"127.0.0.1"` 等) |
| `remotePort` | `Int` | peer port |
| `keepAlive` | `Bool` | HTTP/1.1 default true, `Connection: close` で false |
| `chunked` | `Bool` | `Transfer-Encoding: chunked` の場合 true |

### 3.2 2-arg handler req shape

2-arg form は **streaming** を意図するため、`body` の初期値が 1-arg form と異なります:

| Field | 1-arg handler | 2-arg handler |
|-------|---------------|---------------|
| `body` | span over buffered body (`contentLength` が揃っている) | span `@(start: bodyOffset, len: 0)` (streaming 前提) |
| `contentLength` | 確定値 | header 値をそのまま (>0 でも body は未読) |

他のフィールドは 1-arg と同じ。C26B-023 で上記差異を 3-backend で pin します。

### 3.3 Implementation references

- Interpreter: `src/interpreter/net_eval/h1.rs:1131-1154` (1-arg), `:758-820` (2-arg)
- span pack 構築: `src/interpreter/net_eval/helpers.rs:195-200` (`make_span`, zero-copy)
- Request head parser: `src/interpreter/net_eval/helpers.rs:426-447` (`httpParseRequestHead`)

---

## 4. Span-aware 比較 mold (C26B-016, Option B+)

> **Status**: C26B-016 で land 予定 (Must Fix、Phase 12)。以下は設計 pin。land 後に具体的な source reference を追記します。

以下の public mold を `taida-lang/net` から公開します。3-backend (Interpreter / JS / Native) で parity 保証。

### 4.1 `strOf(span, raw) -> Str`

```
m <= strOf(req.method, req.raw)      // "GET"
```

span を明示的に `Str` に変換します。`Str[req.raw](start <= req.method.start, end <= req.method.start + req.method.len)` の糖衣で、**new allocation を発生させる cold path 用**。

### 4.2 `SpanEquals[span, raw, needle: Str]() -> Bool`

```
SpanEquals[req.method, req.raw, "GET"]()   // Bool
```

span が `needle` と byte-level で一致するか判定します。**zero-copy** (memcmp 相当)、router の method 判定に最適化した **hot path 用**。

### 4.3 `SpanStartsWith[span, raw, prefix: Str]() -> Bool`

```
SpanStartsWith[req.path, req.raw, "/api"]()
```

span の先頭が `prefix` か判定。router pattern match 用。

### 4.4 `SpanSlice[span, raw, start: Int, end: Int]() -> BuchiPack`

```
subSpan <= SpanSlice[req.query, req.raw, 0, 10]()   // @(start, len) pack
```

span の sub-span を返します。`start` / `end` は **親 span 内** の offset (0-based)。query の分解 / header value の部分抽出に使用。

### 4.5 `SpanContains[span, raw, needle: Str]() -> Bool`

```
SpanContains[req.headers(0).value, req.raw, "gzip"]()
```

header 値の中に `needle` が含まれるか判定。`Accept-Encoding` 等の値検索に使用。

### 4.6 使い分け指針

| 用途 | 推奨 mold | 理由 |
|------|----------|------|
| router の method 分岐 (per request) | `SpanEquals` | hot path、allocation を避ける |
| router の path prefix 判定 | `SpanStartsWith` | 同上 |
| log 出力 / デバッグ (cold path) | `strOf` | 1 回だけ allocation、可読性重視 |
| body parsing / JSON 解析 | `strOf(req.body, req.raw)` を JSON mold に渡す | 一度に allocate して再利用 |
| query string 分解 | `SpanSlice` で分解 → 各 subspan に `strOf` | 不要な allocation を避ける |

> **Note**: 上記は C26B-016 Option B+ で land 予定の API surface pin です。実装 land 時に `tests/parity_span_aware_mold.rs` (新規) で 3-backend parity を pin します。

---

## 5. HTTP parse / encode

### 5.1 `httpParseRequestHead(bytes: Bytes) -> Lax[BuchiPack]`

request head (start line + header block、CRLFCRLF まで) を parse。返り値の pack shape は §3 とほぼ同じ (`body` / `bodyOffset` / `contentLength` / `remoteHost` / `remotePort` / `keepAlive` / `chunked` は含まない)。

### 5.2 `httpEncodeResponse(status: Int, headers: @[...], body: Bytes) -> Bytes`

response を wire bytes に encode。詳細は C26B-022 (Step 3 Option B: parser 上限強制) 完了時に追記。

> **C26B-022 Step 3 Option B (Must Fix)**: method 16 byte / path 2048 byte / authority 256 byte 超は parse / encode 双方で `400 Bad Request` を emit (runtime reject)。`-Wformat-truncation` warning-as-error を CI に組み込み、C wire buffer の snprintf truncation を防ぎます。

---

## 6. Client

### 6.1 `httpRequest(url: Str, ?opts: BuchiPack) -> Gorillax[...]`

HTTP client。TLS 自動判定 (`https://` なら TLS)。詳細は C26B-002 FIXED 時に追記。

---

## 7. Known limitations & roadmap

| 項目 | Blocker | Severity | Phase |
|------|---------|----------|-------|
| HTTP/2 parity (3-backend) | C26B-001 | Must Fix | 1 |
| TLS 構成安定化 | C26B-002 | Must Fix | 2 |
| port-bind race 根治 | C26B-003 | **Critical** | 3 |
| throughput regression gate hard-fail | C26B-004 | Must Fix | 4 |
| scatter-gather 24h soak | C26B-005 | Must Fix | 5 |
| HTTP retry shim 撤廃 | C26B-006 | Must Fix | 6 |
| HTTP wire parser 上限強制 | C26B-022 | Must Fix | 12 |
| 2-arg body silent breakage | C26B-023 | Must Fix | 12 |
| span-aware 比較 mold 公開 | C26B-016 | Must Fix | 12 |

WASM バックエンドは gen-C では rejected、D27 送り (`docs/STABILITY.md` §1.2 / §4.2 / §5.2)。

---

## 8. References

- `docs/STABILITY.md` §2.2 / §5.1 — surface 保証範囲と NET stable viewpoint
- `.dev/C26_BLOCKERS.md` — 開発中 blocker の live worklist
- `.dev/C26_DESIGN.md` — Phase 0 Design Lock (Option pin 含む)
- `src/interpreter/net_eval/h1.rs` / `h2.rs` — interpreter reference 実装
- `tests/parity.rs::test_net6_*` — 3-backend parity fixtures
