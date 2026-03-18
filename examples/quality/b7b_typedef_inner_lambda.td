// Quality Test: B-7b — TypeDef メソッド内ラムダ
// メソッド内でラムダ + self フィールド参照

Scorer = @(
  bonus: Int
  calc x: Int =
    addBonus <= _ n = n + bonus
    addBonus(x).toString()
  => :Str
)

s <= Scorer(bonus <= 5)
stdout(s.calc(10))
stdout(s.calc(20))

// Expected: 15
// Expected: 25
