// Quality Test: B-6b — BuchiPack 内パターンマッチ（ラムダ経由）
// フィールド関数内で CondBranch

classifyFn x =
  | x > 0 |> "positive"
  | x == 0 |> "zero"
  | _ |> "negative"
=> :Str

obj <= @(classify <= _ x = classifyFn(x))

stdout(obj.classify(5))
stdout(obj.classify(0))
stdout(obj.classify(0 - 3))
// Expected: positive
// Expected: zero
// Expected: negative
