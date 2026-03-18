// B-3d: Tail recursion from pattern match arm (F-54 regression)

sumTail n acc =
  | n == 0 |> acc
  | _ |> sumTail(n - 1, acc + n)
=> :Int

stdout(sumTail(100, 0).toString())
stdout(sumTail(10000, 0).toString())

// Expected:
// 5050
// 50005000
