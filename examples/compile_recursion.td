// Non-tail recursive factorial (not optimized)
factorial n =
  | n == 1 |> 1
  | _ |> n * factorial(n - 1)
=> :Int
debug(factorial(10))

// Tail-recursive sum (TCO optimized — runs with 100000 depth)
sumTo n acc =
  | n == 0 |> acc
  | _ |> sumTo(n - 1, acc + n)
=> :Int
debug(sumTo(100000, 0))
