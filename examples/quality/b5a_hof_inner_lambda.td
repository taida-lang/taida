// Quality Test: B-5a — HOF 内ラムダ（多段）
// Map のコールバック内でさらにラムダを定義して呼び出す

adder x = (_ y = x + y)(10) => :Int

items <= @[1, 2, 3]

Map[items, _ x = adder(x)]() ]=> result
stdout(result)

// Expected: @[11, 12, 13]
