// W-5g F-1: Result with predicate evaluation
// Result[42, _ = false] should be error (predicate always fails)
stdout(Result[42, _ = false]().toString())

// Result[42, _ = true] should be success (predicate always passes)
stdout(Result[42, _ = true]().toString())

// Result with predicate + unmold success
Result[99, _ = true]() ]=> y
debug(y)
