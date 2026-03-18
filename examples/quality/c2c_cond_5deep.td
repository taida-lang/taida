// Quality Test: C-2c — パターンマッチ 5 段ネスト — 5 段 CondBranch でそれぞれ異なる値を返す
classify x =
  | x > 100 |>
    | x > 500 |>
      | x > 900 |>
        | x > 950 |>
          | x > 990 |> "S+"
          | _ |> "S"
        | _ |> "A+"
      | _ |> "A"
    | _ |> "B"
  | _ |> "C"
=> :Str

stdout(classify(995))
stdout(classify(960))
stdout(classify(910))
stdout(classify(200))
stdout(classify(50))
// Expected:
// S+
// S
// A+
// B
// C
