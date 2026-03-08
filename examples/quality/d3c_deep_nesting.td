// Quality Test: D-3c — 深いネスト
// 深い BuchiPack ネスト（再帰で構築）と深い CondBranch（10段）

// === 20段の BuchiPack ネスト生成 ===
buildNested depth val =
  | depth == 0 |> val
  | _ |> @(inner <= buildNested(depth - 1, val), depth <= depth)
=> :Any

nested <= buildNested(20, "leaf")
stdout("built20")

// 10段も確認
nested10 <= buildNested(10, "leaf")
stdout("built10")

// === 10段パターンマッチ ===
classify x =
  | x >= 90 |> "A+"
  | x >= 80 |> "A"
  | x >= 70 |> "B+"
  | x >= 60 |> "B"
  | x >= 50 |> "C+"
  | x >= 40 |> "C"
  | x >= 30 |> "D"
  | x >= 20 |> "E"
  | x >= 10 |> "F"
  | _ |> "Z"
=> :Str

stdout(classify(95))
stdout(classify(85))
stdout(classify(75))
stdout(classify(65))
stdout(classify(55))
stdout(classify(45))
stdout(classify(35))
stdout(classify(25))
stdout(classify(15))
stdout(classify(5))

// === 多段パターンマッチ（2段ネスト）===
classify2 x y =
  | x > 50 |>
    | y > 50 |> "both high"
    | _ |> "x high"
  | _ |>
    | y > 50 |> "y high"
    | _ |> "both low"
=> :Str

stdout(classify2(60, 60))
stdout(classify2(60, 40))
stdout(classify2(40, 60))
stdout(classify2(40, 40))
