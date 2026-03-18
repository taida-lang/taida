// B-2e: ラムダ内再帰
// capture した変数を使いつつ再帰

step <= 2

countDown n acc =
  | n < 1 |> acc
  | _ |> countDown(n - step, acc + n)
=> :Int

maker <= _ start = countDown(start, 0)

stdout(maker(10).toString())

// countDown(10,0) -> countDown(8,10) -> countDown(6,18)
// -> countDown(4,24) -> countDown(2,28) -> countDown(0,30)
// Expected: 30
