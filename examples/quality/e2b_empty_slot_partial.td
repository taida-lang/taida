// Empty slot partial application tests

add x y =
  x + y
=> :Int

mul x y =
  x * y
=> :Int

sub x y =
  x - y
=> :Int

triple x y z =
  x + y + z
=> :Int

// Case 1: hole at second position
add5 <= add(5, )
stdout(add5(3).toString())

// Case 2: hole at first position
double <= mul(, 2)
stdout(double(7).toString())

// Case 3: both holes
addBoth <= add(, )
stdout(addBoth(10, 20).toString())

// Case 4: middle hole in 3-arg function
f1 <= triple(1, , 3)
stdout(f1(2).toString())

// Case 5: first and last holes
f2 <= triple(, 5, )
stdout(f2(1, 3).toString())

// Case 6: chained partial application result
add10 <= add(10, )
result <= add10(5)
stdout(result.toString())
