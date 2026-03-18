# 命名規則

## 概要

Taida Langでは、コードの可読性と一貫性を保つために以下の命名規則を採用しています。

## 命名規則一覧

| 対象 | 規則 | 例 |
|------|------|-----|
| 型 | PascalCase | `Pilot`, `HttpRequest`, `ValidationError` |
| 変数 | snake_case | `pilot_name`, `total_count`, `is_valid` |
| 関数 | camelCase | `getPilotName`, `calculateTotal`, `isValid` |
| 定数 | UPPER_SNAKE_CASE | `MAX_SIZE`, `DEFAULT_TIMEOUT`, `PI` |
| モジュール | small-kebab-case | `http-client`, `pilot-service`, `json-parser` |
| ファイル | small-kebab-case.td | `http-client.td`, `pilot-service.td` |

---

## 型（PascalCase）

型定義には PascalCase を使用します。

```taida
// 基本的な型定義
Pilot = @(
  name: Str
  age: Int
)

// モールディング型
Mold[T] => Result[T, P <= :T => :Bool] = @(throw: Error)

// エラー型（Errorを継承）
Error => ValidationError = @(field: Str)
Error => HttpError = @(status_code: Int)
```

---

## 変数（snake_case）

変数名には snake_case を使用します。

```taida
// 基本的な変数
pilot_name <= "Misato"
total_count <= 42
is_active <= true

// ぶちパックのフィールド
pilot <= @(
  first_name <= "Misato",
  last_name <= "Katsuragi",
  call_sign <= "Ops-01"
)

// モールディング型のインスタンス
result_pilot <= Result[pilot, _ = true]()
```

---

## 関数（camelCase）

関数名には camelCase を使用します。

```taida
// 基本的な関数
getPilotName pilot: Pilot =
  pilot.first_name + " " + pilot.last_name
=> :Str

// 複数単語の関数名
calculateTotalPrice items: @[Item] =
  Fold[0, items, _ acc item = acc + item.price]() ]=> total
  total
=> :Int

// 真偽値を返す関数（is/has/can プレフィックス）
isValidEmail email: Str =
  // 検証ロジック
=> :Bool

hasPermission pilot: Pilot permission: Str =
  // 権限チェック
=> :Bool
```

---

## 定数（UPPER_SNAKE_CASE）

定数には UPPER_SNAKE_CASE を使用します。

```taida
// 数値定数
MAX_RETRY_COUNT <= 3
DEFAULT_TIMEOUT <= 5000
PI <= 3.14159

// 文字列定数
API_BASE_URL <= "https://api.example.com"
DEFAULT_LOCALE <= "ja-JP"

// 定数のエクスポート
<<< @(MAX_RETRY_COUNT, DEFAULT_TIMEOUT, API_BASE_URL)
```

---

## モジュール・ファイル（small-kebab-case）

モジュール名とファイル名には small-kebab-case を使用します。

```taida
// ファイル: pilot-service.td
>>> ./http-client.td => @(httpGet, httpPost)
>>> ./json-parser.td => @(parseJson)

PilotService = @(
  getPilot id: Int =
    // 実装
  => :Pilot
)

<<< @(PilotService)
```

### ファイル構成例

```
my-project/
  packages.tdm
  main.td
  lib/
    http-client.td
    json-parser.td
    pilot-service.td
  types/
    pilot-types.td
    error-types.td
```

---

## パッケージバージョニング

パッケージバージョンは `@世代.番号.ラベル` 形式を採用しています。

```
@世代.番号.ラベル

世代:   a〜z（単一小文字、破壊的変更で進む）
番号:   1〜（連番、publish ごとにインクリメント）
ラベル: [a-z0-9][a-z0-9-]*（省略可、人間/AI 向けの意味付け）
```

最新判定は `@世代.番号` の比較で完結します。ラベルは順序に影響しません。

```taida
// インポートでのバージョン指定
>>> alice/http@a.5.beta => @(get, post)
>>> bob/utils@b.1 => @(parse)

// エクスポートでのバージョン宣言
<<<@a.1.alpha @(MyApp, Config)
```

例:
- `@a.1` — 最初の公開（ラベルなし）
- `@a.1.alpha` — 初期アルファ
- `@a.5.beta` — ベータ段階
- `@a.12.rc` — リリース候補
- `@b.1.breaking` — 破壊的変更の新世代
- `@x.34.gen-2-stable` — 長いラベルも可

---

## 識別子に使用可能な文字

- 英字（a-z, A-Z）
- 数字（0-9）※先頭以外
- アンダースコア（_）
- Unicode文字（対応予定）

```taida
// OK
pilot_name <= "Misato"
pilot2 <= getPilot(2)
_private <= "internal"

// NG（数字で始まる）
// 2nd_pilot <= ...  // コンパイルエラー
```

---

## 予約語

Taida Langには予約語がありません。`unmold`、`throw` なども関数として扱われます。

```taida
// これらは予約語ではなく、関数/メソッドとして動作
opt.unmold()
error.throw()
```

---

## 推奨事項

### 意味のある名前を使う

```taida
// Good
pilot_count <= pilots.length
active_pilots <= Filter[pilots, _ p = p.is_active]()

// Bad
n <= pilots.length
x <= Filter[pilots, _ p = p.is_active]()
```

### 略語は避ける

```taida
// Good
getPilotById id: Int = ...
calculateTotalPrice items: @[Item] = ...

// Bad
getPltById id: Int = ...
calcTotPrc items: @[Item] = ...
```

### 真偽値には is/has/can プレフィックス

```taida
is_valid <= validateInput(data)
has_permission <= checkPermission(pilot)
can_proceed <= is_valid && has_permission
```
