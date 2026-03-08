// B-1c: 関数内パターンマッチ — 引数で分岐して値を返す
classify n =
  | n > 100 |> "big"
  | n > 10 |> "medium"
  | _ |> "small"
=> :Str

stdout(classify(200))
stdout(classify(50))
stdout(classify(3))
// Expected: big
// Expected: medium
// Expected: small
