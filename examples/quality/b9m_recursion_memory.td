// Quality Test: B-9m — 再帰 + BuchiPack 生成のメモリ使用量
// 1万回ループ相当でメモリが安定することを確認
// SIGSEGV しなければ PASS

buildAndSum n acc =
  | n == 0 |> acc
  | _ |>
    p <= @(val <= n)
    buildAndSum(n - 1, acc + p.val)
=> :Int

stdout(buildAndSum(10000, 0).toString())

// リスト生成の再帰
countDown n acc =
  | n == 0 |> acc
  | _ |> countDown(n - 1, acc + 1)
=> :Int

stdout(countDown(10000, 0).toString())
// Expected:
// 50005000
// 10000
