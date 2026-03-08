// B-3b: Triple-nested pattern match (3 levels deep)

classify x y =
  | x > 0 |>
    | y > 0 |>
      | x > y |> "x-wins"
      | _ |> "y-wins"
    | _ |> "y-neg"
  | _ |> "x-neg"
=> :Str

stdout(classify(5, 3))
stdout(classify(2, 7))
stdout(classify(3, -1))
stdout(classify(-1, 5))

// Expected:
// x-wins
// y-wins
// y-neg
// x-neg
