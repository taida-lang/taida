// Quality Test: B-4c — エラー天井内パターンマッチ
// |== 本体内でパターンマッチ

Error => AppError = @(code: Int)

classify x =
  |== error: Error =
    | error.type == "AppError" |> "app:" + error.message
    | _ |> "other:" + error.message
  => :Str
  | x < 0 |> AppError(type <= "AppError", message <= "neg", code <= 1).throw()
  | x == 0 |> AppError(type <= "AppError", message <= "zero", code <= 2).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(classify(10))
stdout(classify(0))
stdout(classify(-5))

// Expected:
// ok:10
// app:zero
// app:neg
