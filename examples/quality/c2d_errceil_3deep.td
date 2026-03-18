// Quality Test: C-2d — エラー天井 3 段ネスト — 最内 throw → 中間キャッチ → 再 throw → 最外キャッチ
Error => E1 = @(code: Int)
Error => E2 = @(code: Int)

deepThrow x =
  |== outerErr: Error =
    "L3:" + outerErr.message
  => :Str
  |== midErr: Error =
    E2(type <= "E2", message <= "re:" + midErr.message, code <= 2).throw()
  => :Str
  | x < 0 |> E1(type <= "E1", message <= "orig", code <= 1).throw()
  | _ |> "ok:" + x.toString()
=> :Str

stdout(deepThrow(10))
stdout(deepThrow(-1))
// Expected:
// ok:10
// L3:re:orig
