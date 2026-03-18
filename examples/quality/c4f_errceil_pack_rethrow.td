// Quality Test: C-4f — |== handler calls function that creates BuchiPack then re-throws
Error => E1 = @(code: Int)
Error => E2 = @(code: Int)

buildMsg err =
  @(tag <= "mid", detail <= err) ]=> info
  info.tag + ":" + info.detail
=> :Str

test x =
  |== outerErr: Error =
    "outer:" + outerErr.message
  => :Str
  |== midErr: Error =
    E2(type <= "E2", message <= buildMsg(midErr.message), code <= 2).throw()
  => :Str
  | x < 0 |> E1(type <= "E1", message <= "neg", code <= 1).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(test(5))
stdout(test(-1))
// Expected:
// ok:5
// outer:mid:neg
