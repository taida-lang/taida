// Quality Test: B-5b — HOF 内パターンマッチ
// Filter のコールバック内で CondBranch

classify x =
  | Mod[x, 2]().unmold() == 0 |> true
  | _ |> false
=> :Bool

items <= @[1, 2, 3, 4, 5, 6]

Filter[items, _ x = classify(x)]() ]=> evens
stdout(evens)

// Expected: @[2, 4, 6]
