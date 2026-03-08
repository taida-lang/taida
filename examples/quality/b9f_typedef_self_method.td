// Quality Test: B-9f — TypeDef インスタンスが自身を参照するメソッド
// メソッド内で自身のフィールドにアクセスするパターン

Counter = @(
  val: Int
  describe = "Counter:" + val.toString() => :Str
  doubled = val * 2 => :Int
)

c <= Counter(val <= 42)
stdout(c.describe())
stdout(c.doubled().toString())

// メソッドが自身フィールドを使って計算するパターン
Pair = @(
  x: Int
  y: Int
  total = x + y => :Int
)

p <= Pair(x <= 3, y <= 7)
stdout(p.total().toString())
// Expected:
// Counter:42
// 84
// 10
