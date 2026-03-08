// Quality Test: B-6a — BuchiPack 内ラムダフィールド
// 関数を格納して obj.fn(args) で呼び出し

obj <= @(
  double <= _ x = x * 2,
  greet <= _ name = "Hello:" + name,
  add <= _ a b = a + b
)

stdout(obj.double(7).toString())
stdout(obj.greet("Taida"))
stdout(obj.add(3, 4).toString())
// Expected: 14
// Expected: Hello:Taida
// Expected: 7
