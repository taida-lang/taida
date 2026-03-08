// Quality Test: B-7a — TypeDef メソッド内関数定義
// メソッド内でローカル関数を定義して呼び出す

Counter = @(
  base: Int
  compute x: Int =
    double n = n * 2 => :Int
    base + double(x)
  => :Int
)

c <= Counter(base <= 10)
stdout(c.compute(3).toString())
stdout(c.compute(5).toString())

// Expected: 16
// Expected: 20
