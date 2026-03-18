// Quality Test: B-4h — エラー天井内モールド + unmold
// |== 内で Div ]=> して結果を使う

Error => MyError = @(code: Int)

doDiv x y =
  Div[x, y]() ]=> q
  q
=> :Int

compute x y =
  |== error: Error =
    "err:" + error.message
  => :Str
  | y == 0 |> MyError(type <= "MyError", message <= "divzero", code <= 1).throw()
  | _ |> "result:" + doDiv(x, y).toString()
=> :Str

stdout(compute(10, 2))
stdout(compute(10, 0))

// Expected:
// result:5
// err:divzero
