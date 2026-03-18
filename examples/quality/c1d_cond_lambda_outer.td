// Quality Test: C-1d — CondBranch + ラムダ + 後続文で外側変数（F-46 の 3 段版）
outer <= "OUT"

fmt x label = label + ":" + outer + ":" + x.toString() => :Str

classify x =
  mid <= "MID"
  | x > 10 |> fmt(x, mid)
  | x > 0 |> mid + ":" + x.toString()
  | _ |> outer + ":zero"
=> :Str

stdout(classify(20))
stdout(classify(5))
stdout(classify(0))
// Expected:
// MID:OUT:20
// MID:5
// OUT:zero
