// Quality Test: B-7i — TypeDef メソッド内パイプライン
// メソッド内でパイプライン + self フィールド

Transformer = @(
  items: @[Int]
  doubled =
    Map[items, _ x = x * 2]() ]=> result
    Join[result, ","]()
  => :Str
)

t <= Transformer(items <= @[1, 2, 3])
stdout(t.doubled())

// Expected: 2,4,6
