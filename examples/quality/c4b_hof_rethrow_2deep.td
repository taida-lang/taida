// Quality Test: C-4b — HOF callback inner |== + re-throw propagates to outer |==
Error => E1 = @(code: Int)
Error => E2 = @(code: Int)

// Inner function that wraps throw in |== and re-throws
innerGuard x =
  |== innerErr: Error =
    E2(type <= "E2", message <= "re:" + innerErr.message, code <= 2).throw()
  => :Int
  | x < 0 |> E1(type <= "E1", message <= "neg", code <= 1).throw()
  | _ |> x * 10
=> :Int

test items =
  |== outerErr: Error =
    "outer:" + outerErr.message
  => :Str
  Map[items, _ x = innerGuard(x)]() ]=> r
  Join[r, ","]() ]=> v
  v
=> :Str

stdout(test(@[1, 2, 3]))
stdout(test(@[1, -1, 3]))
// Expected:
// 10,20,30
// outer:re:neg
