// Quality Test: C-4c — func + top-level helper + 4-deep nesting with capture
doubleIt x = x * 2 => :Int

f1 a =
  f3 c =
    f4 d =
      f5 e =
        doubleIt(a).toString() + ":" + c.toString() + ":" + d.toString() + ":" + e.toString()
      => :Str
      f5(5)
    => :Str
    f4(4)
  => :Str
  f3(3)
=> :Str

stdout(f1(1))
// Expected: 2:3:4:5
