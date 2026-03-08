// Quality Test: B-9i — 関数から返した List を複数回参照
// retain/release が正しく動くことを確認

makeList n = @[n, n * 2, n * 3] => :@[Int]

xs <= makeList(5)
stdout(xs.length().toString())
stdout(xs)

// 別変数に代入して両方参照
ys <= xs
stdout(ys.length().toString())
stdout(xs)
stdout(ys)
// Expected:
// 3
// @[5, 10, 15]
// 3
// @[5, 10, 15]
// @[5, 10, 15]
