// Quality Test: B-4d — エラー天井内エラー天井（多段）
// ネスト |== で内側の throw を外側がキャッチ

Error => InnerError = @(code: Int)
Error => OuterError = @(code: Int)

inner y =
  |== innerErr: Error =
    "inner:" + innerErr.message
  => :Str
  | y < 0 |> InnerError(type <= "InnerError", message <= "deep", code <= 2).throw()
  | _ |> "ok"
=> :Str

nested x =
  |== outerErr: Error =
    "outer:" + outerErr.message
  => :Str
  | x == 1 |> OuterError(type <= "OuterError", message <= "out", code <= 1).throw()
  | x == 2 |> inner(-1)
  | _ |> "pass"
=> :Str

stdout(nested(0))
stdout(nested(1))
stdout(nested(2))

// Expected:
// pass
// outer:out
// inner:deep
