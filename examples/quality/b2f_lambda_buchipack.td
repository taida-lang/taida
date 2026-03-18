// B-2f: ラムダ内 BuchiPack 生成
// capture 変数を BuchiPack フィールドに代入（F-47 回帰）

team <= "NERV"

makePilot <= _ name = @(name <= name, team <= team)

p <= makePilot("Asuka")
stdout(p.name + ":" + p.team)

// Expected: Asuka:NERV
