// Quality Test: B-6c — BuchiPack 内エラー天井（ラムダ経由）
// フィールド関数内で |==

Error => BadVal = @(message: Str)

safeDoFn x =
  |== error: Error = "caught:" + error.message => :Str
  | x < 0 |> BadVal(type <= "BadVal", message <= "negative").throw()
  | _ |> "ok:" + x.toString()
=> :Str

obj <= @(safeDo <= _ x = safeDoFn(x))

stdout(obj.safeDo(10))
stdout(obj.safeDo(0 - 5))
// Expected: ok:10
// Expected: caught:negative
