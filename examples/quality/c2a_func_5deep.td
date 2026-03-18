// Quality Test: C-2a — 関数 5 段ネスト — f1 → f2 → f3 → f4 → f5 で最内から最外変数を参照
f1 a =
  f2 b =
    f3 c =
      f4 d =
        f5 e =
          a.toString() + ":" + b.toString() + ":" + c.toString() + ":" + d.toString() + ":" + e.toString()
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
// Expected: 1:2:3:4:5
