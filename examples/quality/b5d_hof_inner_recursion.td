// Quality Test: B-5d — HOF 内再帰
// Map のコールバック内で再帰関数を呼び出し

factorial n =
  | n < 2 |> 1
  | _ |> n * factorial(n - 1)
=> :Int

items <= @[1, 2, 3, 4, 5]

Map[items, _ x = factorial(x)]() ]=> result
stdout(result)

// Expected: @[1, 2, 6, 24, 120]
