// Test: same-scope name collision is detected by type checker
// This file should PASS — it demonstrates shadowing (which is allowed)
// while avoiding same-scope redefinition (which is an error).

// Inner scope shadowing is allowed
x <= 10
outer =
  x <= 20
  x
=> :Int
stdout(x.toString())
stdout(outer().toString())

// Different names in same scope — OK
a <= 1
b <= 2
stdout((a + b).toString())

// Variable shadowing in function scope — OK
total <= 100
wrapper =
  total <= 200
  total
=> :Int
stdout(total.toString())
stdout(wrapper().toString())
