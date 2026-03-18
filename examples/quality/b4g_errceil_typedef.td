// Quality Test: B-4g — エラー天井内 TypeDef インスタンス化
// |== 本体内で TypeDef インスタンス

Error => MyError = @(code: Int)
Outcome = @(ok: Bool, value: Str)

attempt x =
  |== error: Error =
    Outcome(ok <= false, value <= error.message)
  => :Outcome
  | x == 0 |> MyError(type <= "MyError", message <= "zero", code <= 0).throw()
  | _ |> Outcome(ok <= true, value <= x.toString())
=> :Outcome

r1 <= attempt(7)
r2 <= attempt(0)
stdout(r1.ok.toString() + ":" + r1.value)
stdout(r2.ok.toString() + ":" + r2.value)

// Expected:
// true:7
// false:zero
