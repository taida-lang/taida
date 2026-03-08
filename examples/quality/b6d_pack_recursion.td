// Quality Test: B-6d — BuchiPack 内再帰
// 外部再帰関数をフィールドラムダから呼び出す

factorial n acc =
  | n == 1 |> acc
  | _ |> factorial(n - 1, n * acc)
=> :Int

obj <= @(
  calc <= _ n = factorial(n, 1)
)

stdout(obj.calc(5).toString())
stdout(obj.calc(1).toString())
stdout(obj.calc(10).toString())
// Expected: 120
// Expected: 1
// Expected: 3628800
