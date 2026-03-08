// Quality Test: B-6g — BuchiPack 内モールド + unmold（ラムダ経由）
// フィールド関数内で ]=> 使用

safeDivFn x y =
  Div[x, y]() ]=> result
  result
=> :Int

obj <= @(safeDiv <= _ x y = safeDivFn(x, y))

stdout(obj.safeDiv(10, 3).toString())
stdout(obj.safeDiv(10, 0).toString())
// Expected: 3
// Expected: 0
