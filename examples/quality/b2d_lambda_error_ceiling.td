// B-2d: ラムダ内エラー天井
// |== 内から外側変数を参照

Error => MyError = @(message: Str)
fallback <= "default"

safeDo x =
  |== error: Error = fallback => :Str
  | x > 0 |> "ok:" + x.toString()
  | _ |> MyError(type <= "MyError", message <= "bad").throw()
=> :Str

runner <= _ val = safeDo(val)

stdout(runner(5))
stdout(runner(0))

// Expected:
// ok:5
// default
