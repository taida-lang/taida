// Quality Test: B-6i — BuchiPack フィールド + 外部変数 capture
// フィールドラムダから外側変数を参照

prefix <= "["
suffix <= "]"
multiplier <= 10

obj <= @(
  wrap <= _ text = prefix + text + suffix,
  scale <= _ x = x * multiplier
)

stdout(obj.wrap("hello"))
stdout(obj.scale(5).toString())
// Expected: [hello]
// Expected: 50
