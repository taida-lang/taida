// B-1d: 関数内エラー天井 — |== でエラーを捕捉して結果を返す
Error => BadInput = @(message: Str)

safeParse input =
  |== error: Error =
    "error:" + error.message
  => :Str
  | input == "" |> BadInput(type <= "BadInput", message <= "empty").throw()
  | _ |> "ok:" + input
=> :Str

stdout(safeParse("hello"))
stdout(safeParse(""))
// Expected: ok:hello
// Expected: error:empty
