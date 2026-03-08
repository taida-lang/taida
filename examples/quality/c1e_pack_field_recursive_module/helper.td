// C-1e helper: BuchiPack フィールド関数がモジュール内の別関数を再帰呼び出し（F-56 + B-6d 複合）

countdown n acc =
  | n == 0 |> acc
  | _ |> countdown(n - 1, acc + n.toString() + ",")
=> :Str

tools <= @(
  run <= _ n = countdown(n, "")
)

<<< @(tools)
