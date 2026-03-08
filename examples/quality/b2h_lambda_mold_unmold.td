// B-2h: ラムダ内モールド + unmold
// ラムダ内で ]=> 使用

safeDivide x y =
  Div[x, y]() ]=> result
  result
=> :Int

runner <= _ x = safeDivide(x, 3)

stdout(runner(10).toString())
stdout(runner(0).toString())

// Expected:
// 3
// 0
