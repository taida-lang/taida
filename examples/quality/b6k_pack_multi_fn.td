// Quality Test: B-6k — BuchiPack 5+ 関数フィールド
// 5 つ以上の関数フィールドを持つ BuchiPack で全て呼び出し（F-60 回帰）

obj <= @(
  f1 <= _ x = x + 1,
  f2 <= _ x = x + 2,
  f3 <= _ x = x + 3,
  f4 <= _ x = x + 4,
  f5 <= _ x = x + 5,
  f6 <= _ x = x + 6
)

stdout(obj.f1(0).toString())
stdout(obj.f2(0).toString())
stdout(obj.f3(0).toString())
stdout(obj.f4(0).toString())
stdout(obj.f5(0).toString())
stdout(obj.f6(0).toString())
// Expected: 1
// Expected: 2
// Expected: 3
// Expected: 4
// Expected: 5
// Expected: 6
