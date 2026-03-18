// B-1e: 関数内ネスト有り再帰 — 再帰中に BuchiPack を生成・分解
makePair n = @(val <= n, rest <= n - 1) => :@(val: Int, rest: Int)

buildSum n acc =
  | n == 0 |> acc
  | _ |>
    buildSum(makePair(n).rest, acc + makePair(n).val)
=> :Int

stdout(buildSum(5, 0).toString())
// Expected: 15
