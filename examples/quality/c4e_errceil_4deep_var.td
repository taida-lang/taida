// Quality Test: C-4e — 4-deep |== re-throw + outer variable reference
Error => E1 = @(code: Int)
Error => E2 = @(code: Int)
Error => E3 = @(code: Int)

test x tag =
  |== e4: Error =
    "L4:" + tag + ":" + e4.message
  => :Str
  |== e3: Error =
    E3(type <= "E3", message <= "r3:" + e3.message, code <= 3).throw()
  => :Str
  |== e2: Error =
    E2(type <= "E2", message <= "r2:" + e2.message, code <= 2).throw()
  => :Str
  | x < 0 |> E1(type <= "E1", message <= "orig", code <= 1).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(test(10, "T"))
stdout(test(-1, "T"))
// Expected:
// ok:10
// L4:T:r3:r2:orig
