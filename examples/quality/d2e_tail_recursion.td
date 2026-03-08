// D-2e: docs/reference/tail_recursion.md code examples

// Tail recursive sum
sumTail n: Int acc: Int =
  | n < 1 |> acc
  | _ |> sumTail(n - 1, acc + n)
=> :Int

sumTo n: Int =
  sumTail(n, 0)
=> :Int

stdout(sumTo(100).toString())
stdout(sumTo(1000).toString())

// Tail recursive factorial
factTail n: Int acc: Int =
  | n < 2 |> acc
  | _ |> factTail(n - 1, acc * n)
=> :Int

factorial n: Int =
  factTail(n, 1)
=> :Int

stdout(factorial(10).toString())

// Tail recursive fibonacci
fibTail n: Int a: Int b: Int =
  | n == 0 |> a
  | n == 1 |> b
  | _ |> fibTail(n - 1, b, a + b)
=> :Int

fib n: Int =
  fibTail(n, 0, 1)
=> :Int

stdout(fib(10).toString())
stdout(fib(20).toString())

// Large number to verify TCO (no stack overflow)
stdout(sumTo(100000).toString())
