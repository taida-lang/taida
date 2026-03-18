// Quality Test: C-1c — |== ネスト + 内側 throw を外側がキャッチ + 外側変数参照（F-49 + B-4d 複合）
Error => Inner = @(code: Int)
Error => Outer = @(code: Int)
tag <= "CTX"

innerOp x =
  |== innerErr: Error =
    tag + ":inner:" + innerErr.message
  => :Str
  | x < 0 |> Inner(type <= "Inner", message <= "neg", code <= 1).throw()
  | _ |> tag + ":ok:" + x.toString()
=> :Str

outerOp x =
  |== outerErr: Error =
    tag + ":outer:" + outerErr.message
  => :Str
  | x == 0 |> Outer(type <= "Outer", message <= "zero", code <= 2).throw()
  | _ |> innerOp(x)
=> :Str

stdout(outerOp(5))
stdout(outerOp(0))
stdout(outerOp(-1))
// Expected:
// CTX:ok:5
// CTX:outer:zero
// CTX:inner:neg
