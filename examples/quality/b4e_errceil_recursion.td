// Quality Test: B-4e — エラー天井内再帰
// |== 本体内で再帰（throw で脱出）

Error => LimitError = @(code: Int)

countUp n limit =
  |== error: Error =
    "stopped:" + error.message
  => :Str
  | n >= limit |> LimitError(type <= "LimitError", message <= n.toString(), code <= 1).throw()
  | _ |> countUp(n + 1, limit)
=> :Str

stdout(countUp(0, 3))

// Expected:
// stopped:3
