// D-1l: docs/guide/11_async.md code examples
// NOTE: Async only works in Interpreter. JS/Native will error.

// Basic async
delay ms: Int =
  Async[ms]()
=> :Async[Int]

result <= delay(100)
result ]=> val
stdout(val.toString())
