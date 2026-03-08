// Quality Test: B-5g — HOF 内モールド + unmold
// Map のコールバック内で Div + unmold 使用

safeDiv x = Div[x, 3]().unmold() => :Int

items <= @[10, 20, 30, 40]

Map[items, _ x = safeDiv(x)]() ]=> result
stdout(result)

// Expected: @[3, 6, 10, 13]
