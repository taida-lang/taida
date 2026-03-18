# ぶちパック構文

> **PHILOSOPHY.md -- II.** だいじなものはふくろにしまっておきましょう

---

## ぶちパックとは

名前付きフィールドの集合を表現する構造化データ記法です。関連するデータをひとまとめにして名前をつけ、安全に持ち運びます。ふくろにしまう、ということです。

```taida
pilot <= @(name <= "Asuka", age <= 14, active <= true)
```

---

## 基本構文

### 値の作成

```taida
pack <= @(name <= "Asuka", age <= 14, active <= true)
```

### クラスライク/型定義

型を定義するときは `=` を使います:

```taida
Pilot = @(
  name: Str
  age: Int
  active: Bool
)

// インスタンス化
rei <= Pilot(name <= "Rei", age <= 14, active <= true)
```

### フィールドアクセス

```taida
pilot <= @(name <= "Shinji", age <= 14, callSign <= "Ikari")

pilotName <= pilot.name      // "Shinji"
pilotAge <= pilot.age        // 14
pilotCall <= pilot.callSign  // "Ikari"
```

### 存在しないフィールド

存在しないフィールドにアクセスするとコンパイルエラーになります:

```taida
pilot <= @(name <= "Rei")
syncRate <= pilot.syncRate  // コンパイルエラー: フィールド 'syncRate' は存在しません
```

型で定義されたフィールドはデフォルト値を持ちます:

```taida
Pilot = @(name: Str, callSign: Str, age: Int)
rei <= Pilot(name <= "Rei")  // callSign = "", age = 0
```

---

## 即時評価

ぶちパックのフィールドは代入時に即座に評価されます。

```taida
data <= @(
  timestamp <= getCurrentTime(),    // 代入時に即座に評価されます
  computed <= heavyCalculation()    // 代入時に即座に評価されます
)

// この時点で全フィールドは評価済みです
ts <= data.timestamp  // キャッシュされた値を取得します
```

これはスコープベース管理と組み合わさって安全性を保証します。参照先が解放されても、値はコピーされているため問題ありません。

---

## スコープベース管理

全てのぶちパック変数は、定義されたスコープで自動管理されます:

```taida
config <= @(host <= "localhost", port <= 8080)  // モジュールスコープ

processData items =
  temp <= @(processed <= items.length(), timestamp <= now())
  result <= @(count <= temp.processed, time <= temp.timestamp)
  result  // 戻り値として返されます
=> :@(count: Int, time: Str)
// temp は関数終了時に自動解放されます
```

---

## 型の継承（InheritanceDef）

`=>` 演算子で型の継承を表現します:

```taida
Pilot = @(
  name: Str
  age: Int
)

// Pilot を継承します
Pilot => NervStaff = @(
  department: Str
  rank: Int
)

staff <= NervStaff(
  name <= "Ritsuko",
  age <= 30,
  department <= "Science",
  rank <= 2
)

// 親フィールドにもアクセスできます
staff.name        // "Ritsuko"
staff.department  // "Science"
```

### メソッド内でのフィールドアクセス

メソッド内では、親フィールドも子フィールドも区別なく直接アクセスできます。`self` や `super` は不要です:

```taida
Pilot = @(
  name: Str
  age: Int
)

Pilot => NervOfficer = @(
  rank: Int
  intro =
    `I'm ${name}, rank ${Str[rank]() ]=> _}, age ${Str[age]() ]=> _}.`
  => :Str
)

ritsuko <= NervOfficer(name <= "Ritsuko", age <= 30, rank <= 2)
greeting <= ritsuko.intro()  // "I'm Ritsuko, rank 2, age 30."
```

---

## 構造的部分型付け

Taida は構造的部分型付けを採用しています。必要なフィールドがあれば互換と見なされ、余分なフィールドは許容されます。

```taida
// HasName 型は name フィールドだけを要求します
HasName = @(name: Str)

greet person: HasName =
  stdout("Hello, " + person.name)
=> :Void

// Pilot は name フィールドを持つので、HasName として渡せます
pilot <= @(name <= "Asuka", age <= 14, department <= "Unit-02")
greet(pilot)  // "Hello, Asuka"
```

---

## ネスト構造

ぶちパックは直接ネストできます:

```taida
pilot <= @(
  name <= "Rei",
  contact <= @(
    email <= "rei@nerv.jp",
    phone <= "NERV-001"
  )
)

email <= pilot.contact.email  // "rei@nerv.jp"
```

### 型定義でのネスト

```taida
Pilot = @(
  name: Str
  age: Int
  base: @(
    city: Str
    zip: Str
    org: @(
      code: Str
      name: Str
    )
  )
)

pilot <= Pilot(
  name <= "Shinji"
  age <= 14
  base <= @(
    city <= "Tokyo-3"
    zip <= "999-0003"
    org <= @(code <= "NERV", name <= "NERV HQ")
  )
)

orgCode <= pilot.base.org.code  // "NERV"
```

---

## 関数を含むぶちパック

ぶちパック内に関数を定義できます:

```taida
mathUtils <= @(
  add x: Int y: Int = x + y => :Int,
  subtract x: Int y: Int = x - y => :Int,
  PI <= 3.14159
)

result <= mathUtils.add(3, 5)  // 8
pi <= mathUtils.PI             // 3.14159
```

---

## 関数への名前付き引数

```taida
connect host: Str port: Int timeout: Int =
  // 接続処理
=> :Connection

options <= @(host <= "localhost", port <= 8080, timeout <= 5000)
conn <= connect(options)  // パラメータが自動展開されます
```

---

## モールディング型との統合

ぶちパック内にモールディング型を格納できます:

```taida
pilotData <= @(
  name <= "Asuka",
  syncRate <= Div[780, 10]()
)

// ]=> でアンモールディング
pilotData.syncRate ]=> rateValue  // 78
```

モールディング型の詳細は [モールディング型](05_molding.md) を参照してください。

---

## リストリテラル `@[...]`

リストは `@[...]` で表現します:

```taida
numbers <= @[1, 2, 3, 4, 5]
names <= @["Asuka", "Shinji", "Rei"]
empty: @[Int] <= @[]

// 型付きリスト
pilots: @[Pilot] <= @[
  Pilot(name <= "Asuka", age <= 14),
  Pilot(name <= "Shinji", age <= 14)
]
```

---

## JSON からの変換

外部の JSON データをぶちパックに変換するには、`JSON[raw, Schema]()` モールドを使います。詳細は [JSON 溶鉄](03_json.md) を参照してください。

```taida
Pilot = @(name: Str, age: Int)

raw <= '{"name": "Ritsuko", "age": 30}'
JSON[raw, Pilot]() ]=> pilot
// pilot: @(name <= "Ritsuko", age <= 30)
```

---

## まとめ

| 概念 | 構文 |
|------|------|
| 値の作成 | `@(field <= value, ...)` |
| 型定義（TypeDef） | `TypeName = @(field: Type, ...)` |
| インスタンス化 | `TypeName(field <= value, ...)` |
| フィールドアクセス | `pack.fieldName` |
| 型の継承（InheritanceDef） | `Parent => Child = @(...)` |
| ネスト | `@(inner <= @(field <= value))` |
| リストリテラル | `@[elem1, elem2, ...]` |
| JSON 変換 | `JSON[raw, TypeName]() ]=> val` |
