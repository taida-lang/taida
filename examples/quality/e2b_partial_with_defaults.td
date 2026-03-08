sum3 a: Int b: Int <= 10 c: Int <= a + b =
  a + b + c
=> :Int

add_from_1 <= sum3(1, )
stdout(add_from_1(2).toString())

fill_middle <= sum3(, 2)
stdout(fill_middle(3).toString())
