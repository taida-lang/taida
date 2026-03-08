// D-1j: docs/guide/09_functions.md code examples

// Basic functions
double x: Int =
  x * 2
=> :Int

add x: Int y: Int =
  x + y
=> :Int

stdout(double(5).toString())
stdout(add(3, 5).toString())

// Multi-line function
processData data: Str =
  cleaned <= Trim[data]()
  upper <= Upper[cleaned]()
  upper
=> :Str

stdout(processData("  hello world  "))

// No-arg function
getVersion =
  "1.0.0"
=> :Str

stdout(getVersion())

// Return BuchiPack
createProfile name: Str age: Int =
  @(name <= name, age <= age, active <= true)
=> :@(name: Str, age: Int, active: Bool)

profile <= createProfile("Asuka", 14)
stdout(profile.name)
stdout(profile.active.toString())

// Lambda functions
numbers <= @[1, 2, 3, 4, 5]
Map[numbers, _ x = x * 2]() ]=> doubled
stdout(doubled)
Filter[numbers, _ x = x > 3]() ]=> filtered
stdout(filtered)
Fold[numbers, 0, _ acc x = acc + x]() ]=> total
stdout(total.toString())

// Forward pipeline
"  hello world  " => Trim[_]() => Upper[_]() => pipe_result
stdout(pipe_result)

// Pipeline with molds
"  Asuka Langley  " => Trim[_]() => Upper[_]() => Split[_, " "]() => pipe_result2
stdout(pipe_result2)

// Numeric pipeline
@[3, 1, 4, 1, 5, 9, 2, 6] => Sort[_]() => Unique[_]() => Filter[_, _ x = x > 3]() => pipe_result3
stdout(pipe_result3)

// Recursive
factorial n: Int =
  | n < 2 |> 1
  | _ |> n * factorial(n - 1)
=> :Int

stdout(factorial(5).toString())

// Closures
makeGreeter prefix: Str =
  _ name = prefix + name
=> :Str

greet <= makeGreeter("Hello, ")
stdout(greet("Taida"))
