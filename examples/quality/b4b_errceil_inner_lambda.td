// Quality Test: B-4b — エラー天井内ラムダ
// |== 本体内でラムダを定義・外側変数を参照

Error => MyError = @(code: Int)
prefix <= "ERR"

handle x =
  |== error: Error =
    fmt <= _ msg = prefix + ":" + msg
    fmt(error.message)
  => :Str
  | x == 0 |> MyError(type <= "MyError", message <= "zero", code <= 0).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(handle(3))
stdout(handle(0))

// Expected:
// ok:3
// ERR:zero
