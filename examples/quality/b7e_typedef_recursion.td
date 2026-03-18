// Quality Test: B-7e — TypeDef メソッド内再帰
// メソッドからローカル関数の再帰呼び出し

Factorial = @(
  n: Int
  compute =
    go i acc =
      | i == 0 |> acc
      | _ |> go(i - 1, acc * i)
    => :Int
    go(n, 1).toString()
  => :Str
)

f <= Factorial(n <= 5)
stdout(f.compute())

// Expected: 120
