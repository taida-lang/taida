// Quality Test: B-7f — TypeDef メソッド内 BuchiPack 生成
// self フィールドを含む BuchiPack を返す

Profile = @(
  name: Str
  age: Int
  toRecord =
    @(label <= name, years <= age)
  => :@(label: Str, years: Int)
)

p <= Profile(name <= "Rei", age <= 14)
rec <= p.toRecord()
stdout(rec.label)
stdout(rec.years.toString())

// Expected: Rei
// Expected: 14
