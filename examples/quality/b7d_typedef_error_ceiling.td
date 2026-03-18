// Quality Test: B-7d — TypeDef メソッド内エラー天井
// メソッド内で |== を使ったエラーハンドリング

Error => LimitError = @(code: Int)

Checker = @(
  limit: Int
  check x: Int =
    |== err: Error =
      "caught:" + err.message
    => :Str
    | x > limit |> LimitError(type <= "LimitError", message <= "over limit", code <= 1).throw()
    | _ |> "ok:" + x.toString()
  => :Str
)

c <= Checker(limit <= 10)
stdout(c.check(5))
stdout(c.check(20))

// Expected: ok:5
// Expected: caught:over limit
