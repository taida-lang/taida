// Quality Test: B-7h — TypeDef メソッド内モールド + unmold
// メソッド内で ]=> を使用

SafeMath = @(
  x: Int
  y: Int
  divide =
    Div[x, y]() ]=> result
    result.toString()
  => :Str
)

m <= SafeMath(x <= 10, y <= 3)
stdout(m.divide())

// Expected: 3
