// Quality Test: B-5e — HOF 内 BuchiPack 生成
// Map で各要素を BuchiPack に変換

items <= @[1, 2, 3]

Map[items, _ x = @(val <= x, doubled <= x * 2)]() ]=> packs
stdout(packs.length().toString())
stdout(packs.first().unmold().val.toString())
stdout(packs.first().unmold().doubled.toString())
stdout(packs.last().unmold().val.toString())
stdout(packs.last().unmold().doubled.toString())

// Expected: 3
// Expected: 1
// Expected: 2
// Expected: 3
// Expected: 6
