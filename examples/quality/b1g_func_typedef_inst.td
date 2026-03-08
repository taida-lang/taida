// B-1g: 関数内 TypeDef インスタンス化 — TypeDef を定義してインスタンスを返す
Pilot = @(name: Str, score: Int)

makePilot name score =
  Pilot(name <= name, score <= score)
=> :Pilot

p <= makePilot("Rei", 95)
stdout(p.name)
stdout(p.score.toString())
// Expected: Rei
// Expected: 95
