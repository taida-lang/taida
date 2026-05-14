# `taida-lang/pool` API リファレンス

`taida-lang/pool` はリソースプーリングの最小契約を提供するコア同梱
パッケージです。HTTP クライアントコネクション、DB ドライバ接続、その他
共有資源のプール化を必要とする場面で、生成・取得・返却・破棄の共通 API
を提供します。

```taida
>>> taida-lang/pool => @(poolCreate, poolAcquire, poolRelease, poolClose, poolHealth)
```

ドライバごとの接続確立・検証ポリシーは `taida-lang/pool` 自体には
含まれません。上位ライブラリでプール化対象 (`Molten` ハンドル) を
具体化する設計です。

---

## 1. プール生成

### 1.1 `poolCreate`

```
poolCreate(config: BuchiPack) -> Result[@(pool: Pool), _]
```

設定 `config` を受け取り、プールを作成します。

`config` で受け付けるフィールドは次のとおりです。すべて省略可能で、
省略時はそれぞれの既定値が適用されます。

| フィールド | 型 | 省略時のデフォルト | 意味 |
|------------|----|--------------------|------|
| `maxSize` | `Int` | `10` | 同時保持できるリソースの最大数 |
| `maxIdle` | `Int` | `maxSize` | アイドル保持の上限 |
| `acquireTimeoutMs` | `Int` | `30000` | 取得待ちのタイムアウト (ミリ秒) |

- `maxSize` は `1` 以上の正の整数が必要です。`0` 以下を渡すと失敗
  `Result` を返します。
- `maxIdle` は `0` 以上で、`maxSize` を超えてはいけません。
- `acquireTimeoutMs` は `1` 以上の正の整数が必要です。
- 戻り値の `pool` フィールドは不透明ハンドルです。後続の API
  (`poolAcquire` 等) にそのまま渡してください。

```taida
poolCreate(@(maxSize <= 8, acquireTimeoutMs <= 5000)) ]=> created
pool <= created.pool
```

---

## 2. リソース取得

### 2.1 `poolAcquire`

```
poolAcquire(pool: Pool) -> Async[Result[@(resource, token), _]]
poolAcquire(pool: Pool, timeoutMs: Int) -> Async[Result[@(resource, token), _]]
```

プールからリソースを取得する非同期処理を返します。

- `timeoutMs` を省略した場合は `poolCreate` 時の `acquireTimeoutMs` が
  使用されます。明示する場合は `1` 以上の正の整数を渡してください。
- 戻り値の `resource` は取り出したリソース本体、`token` は `poolRelease`
  に渡す返却用ハンドルです。
- プールが閉じられている場合は `Result` の失敗側に `kind: "closed"`
  が入ります。
- タイムアウトした場合は `Result` の失敗側に `kind: "timeout"` が
  入ります。

```taida
poolAcquire(pool, 2000) ]=> acquired
acquired |==
  | @(has_value <= true) <= |
      r <= acquired.value
      ... r.resource を使った処理 ...
      poolRelease(pool, r.token, r.resource) ]=> _
  | _ <= stdout("acquire failed: " + acquired.kind)
```

---

## 3. リソース返却

### 3.1 `poolRelease`

```
poolRelease(pool: Pool, token: Token, resource: Resource) -> Result[@(ok: Bool, reused: Bool), _]
```

`poolAcquire` で取得したリソースをプールに返却します。

- `token` は `poolAcquire` の戻り値に含まれていたものをそのまま渡し
  ます。
- `resource` は取得時のリソース本体です。返却前にリソースを破棄して
  しまった場合は別のリソースを渡しても構いませんが、プール側の
  内部整合性のため新規取得を推奨します。
- 戻り値の `ok` は返却が成功したか、`reused` は返却したリソースが
  アイドルキューに再投入されたか (`true`) 廃棄されたか (`false`) を
  示します。

```taida
poolRelease(pool, acquired.token, acquired.resource) ]=> released
| released.reused
  |> stdout("再利用キューへ戻った")
  | _ |> stdout("リソースは破棄された")
```

---

## 4. プール破棄

### 4.1 `poolClose`

```
poolClose(pool: Pool) -> Async[Result[@(ok: Bool), _]]
```

プールを閉じ、保持しているすべてのアイドルリソースを破棄します。
進行中の `poolAcquire` 待ちは失敗 (`kind: "closed"`) で完了します。

- 同一プールを 2 回以上 `poolClose` しても失敗にはなりません
  (冪等)。

```taida
poolClose(pool) ]=> closed
stdout("closed: " + closed.ok.toString())
```

---

## 5. プールの状態取得

### 5.1 `poolHealth`

```
poolHealth(pool: Pool) -> @(open: Bool, idle: Int, inUse: Int, waiting: Int)
```

プールの現在状態を観測します。同期処理で、即座にスナップショットを
返します。

| フィールド | 型 | 意味 |
|------------|----|------|
| `open` | `Bool` | プールが利用可能か (`poolClose` 後は `false`) |
| `idle` | `Int` | アイドルキューに保持されているリソース数 |
| `inUse` | `Int` | 現在貸し出し中のリソース数 |
| `waiting` | `Int` | 取得待ちを起こしている呼び出し数 |

```taida
health <= poolHealth(pool)
stdout("idle: " + health.idle.toString() + ", in use: " + health.inUse.toString())
```

監視 / メトリクス送信のための観測ポイントとしての利用を想定しています。
`waiting > 0` が継続する場合は `maxSize` の引き上げや上位レイヤーの
バックプレッシャ設計の見直しが必要です。

---

## 6. バックエンド対応

| バックエンド | 対応範囲 |
|--------------|----------|
| インタプリタ | 全 API |
| ネイティブ | 全 API |
| JS | 全 API |
| WASM (`wasm-min` / `wasm-wasi` / `wasm-edge`) | 利用不可 |
| WASM (`wasm-full`) | 全 API |

`wasm-min` / `wasm-wasi` / `wasm-edge` プロファイルではプール本体の
ランタイムスレッドモデルが利用できないため、インポート自体が拒否
されます。詳細は
[`docs/reference/wasm_profiles.md`](../reference/wasm_profiles.md) を
参照してください。

---

## 関連リファレンス

- [`bundled_packages.md`](bundled_packages.md) — コア同梱パッケージの入口
- [`docs/reference/standard_library.md`](../reference/standard_library.md) — `Result` / `Async` / `Lax` の仕様
- [`docs/guide/11_async.md`](../guide/11_async.md) — `Async[T]` と `]=>` の使い方
