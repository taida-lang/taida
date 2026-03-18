// B-3a: Pattern match inner lambda
// Lambda defined outside, called inside condition arm

tripler <= _ x = x * 3
doubler <= _ x = x * 2

pick n =
  | n == 1 |> tripler(n)
  | n == 2 |> doubler(n)
  | _ |> (_ x = x + 100)(n)
=> :Int

stdout(pick(1).toString())
stdout(pick(2).toString())
stdout(pick(3).toString())

// Expected:
// 3
// 4
// 103
