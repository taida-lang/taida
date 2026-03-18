// Quality Test: C-4d — 6-deep function nesting capture boundary
f1 a =
  f2 b =
    f3 c =
      f4 d =
        f5 e =
          f6 f =
            a.toString() + ":" + b.toString() + ":" + c.toString() + ":" + d.toString() + ":" + e.toString() + ":" + f.toString()
          => :Str
          f6(6)
        => :Str
        f5(5)
      => :Str
      f4(4)
    => :Str
    f3(3)
  => :Str
  f2(2)
=> :Str

stdout(f1(1))
// Expected: 1:2:3:4:5:6
