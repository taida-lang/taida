// Quality Test: B-5i — HOF + 外部変数 capture
// コールバックから外側変数を参照（F-41/43 回帰テスト）

prefix <= "item:"
multiplier <= 10

items <= @[1, 2, 3]

Map[items, _ x = prefix + (x * multiplier).toString()]() ]=> result
stdout(result)

// Expected: @[item:10, item:20, item:30]
