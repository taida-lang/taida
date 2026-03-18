// Quality Test: D-3d — 10万回末尾再帰
// TCO が正しく動作していれば問題なく完走するはず

// 単純カウントダウン
countdown n =
  | n == 0 |> "done"
  | _ |> countdown(n - 1)
=> :Str

stdout(countdown(100000))

// 累積加算
sumTo n acc =
  | n == 0 |> acc
  | _ |> sumTo(n - 1, acc + n)
=> :Int

result <= sumTo(100000, 0)
stdout(result.toString())
