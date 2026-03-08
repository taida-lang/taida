// Quality Test: B-4a — エラー天井内関数定義
// |== 本体内でローカル関数を定義・呼び出し

Error => MyError = @(code: Int)

process x =
  |== error: Error =
    helper msg = "caught:" + msg => :Str
    helper(error.message)
  => :Str
  | x < 0 |> MyError(type <= "MyError", message <= "negative", code <= 1).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(process(5))
stdout(process(-1))

// Expected:
// ok:5
// caught:negative
