// Quality Test: B-9d — closure 相互参照
// funcA が funcB を、funcB が funcA を参照するパターン
// Taida は関数定義順に依存 — 前方参照が可能か確認

isEven n =
  | n == 0 |> true
  | _ |> isOdd(n - 1)
=> :Bool

isOdd n =
  | n == 0 |> false
  | _ |> isEven(n - 1)
=> :Bool

stdout(isEven(4).toString())
stdout(isOdd(3).toString())
stdout(isEven(7).toString())
// Expected:
// true
// true
// false
