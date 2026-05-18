// === Taida Lang: Functions ===

// Basic function
add x y =
  x + y
=> :Int

result <= add(3, 5)
stdout(`3 + 5 = ${result}`)

// Multi-line function
greet name greeting =
  message <= `${greeting}, ${name}!`
  message
=> :Str

stdout(greet("Taida", "Hello"))

// Tail-recursive factorial
factorialLoop n acc =
  | n < 2 |> acc
  | _ |> factorialLoop(n - 1, acc * n)
=> :Int

factorial n =
  factorialLoop(n, 1)
=> :Int

stdout(`5! = ${factorial(5)}`)
stdout(`10! = ${factorial(10)}`)

// Tail-recursive Fibonacci
fibLoop n a b =
  | n == 0 |> a
  | n == 1 |> b
  | _ |> fibLoop(n - 1, b, a + b)
=> :Int

fib n =
  fibLoop(n, 0, 1)
=> :Int

stdout(`fib(10) = ${fib(10)}`)
