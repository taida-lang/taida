// B-2g: ラムダ内 TypeDef インスタンス化
// capture 変数を TypeDef フィールドに渡す

Pilot = @(name: Str, score: Int)
bonus <= 10

makePilot name base =
  Pilot(name <= name, score <= base + bonus)
=> :Pilot

runner <= _ n = makePilot(n, 90)

p <= runner("Rei")
stdout(p.name + ":" + p.score.toString())

// Expected: Rei:100
