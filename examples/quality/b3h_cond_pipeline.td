// B-3h: Pipeline inside pattern match arm
// QF-4/QF-5 fixed: multi-statement block in cond arm body

double x = x * 2 => :Int
addOne x = x + 1 => :Int

// Nested function calls (original test)
compute flag n =
  | flag |> addOne(double(n))
  | _ |> double(addOne(n))
=> :Int

stdout(compute(true, 5).toString())
stdout(compute(false, 5).toString())

// Multi-statement block in cond arm body (QF-4 fix)
computeBlock flag n =
  | flag |>
    doubled <= double(n)
    addOne(doubled)
  | _ |>
    incremented <= addOne(n)
    double(incremented)
=> :Int

stdout(computeBlock(true, 5).toString())
stdout(computeBlock(false, 5).toString())

// Expected:
// 11
// 12
// 11
// 12
