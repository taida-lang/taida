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

// Recursive factorial
factorial n =
  | n < 2 |> 1
  | _ |> n * factorial(n - 1)
=> :Int

stdout(`5! = ${factorial(5)}`)
stdout(`10! = ${factorial(10)}`)

// Fibonacci
fib n =
  | n == 0 |> 0
  | n == 1 |> 1
  | _ |> fib(n - 1) + fib(n - 2)
=> :Int

stdout(`fib(10) = ${fib(10)}`)
