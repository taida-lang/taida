// Quality Test: B-7c — TypeDef メソッド内パターンマッチ
// self フィールドでパターンマッチ

Classifier = @(
  value: Int
  classify =
    | value > 0 |> "positive"
    | value == 0 |> "zero"
    | _ |> "negative"
  => :Str
)

a <= Classifier(value <= 42)
b <= Classifier(value <= 0)
c <= Classifier(value <= -3)
stdout(a.classify())
stdout(b.classify())
stdout(c.classify())

// Expected: positive
// Expected: zero
// Expected: negative
