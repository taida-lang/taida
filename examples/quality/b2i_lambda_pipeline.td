// B-2i: ラムダ内パイプライン
// ラムダ内でパイプライン + 外側変数使用

double x = x * 2 => :Int
addOne x = x + 1 => :Int

transform x =
  x => double(_) => addOne(_)
=> :Int

runner <= _ v = transform(v)

stdout(runner(5).toString())

// 5 -> double -> 10 -> addOne -> 11
// Expected: 11
