// B-3c: Error ceiling inside pattern match arm

Error => MyError = @(code: Int)

process n =
  |== error: Error =
    "caught:" + error.message
  => :Str
  | n == 0 |> MyError(type <= "MyError", message <= "zero", code <= 0).throw()
  | _ |> "ok:" + n.toString()
=> :Str

stdout(process(5))
stdout(process(0))

// Expected:
// ok:5
// caught:zero
