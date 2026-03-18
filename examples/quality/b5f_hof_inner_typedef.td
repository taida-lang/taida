// Quality Test: B-5f — HOF 内 TypeDef インスタンス化
// Map で各要素を TypeDef インスタンスに変換

Item = @(id: Int, label: Str)

makeItem name = Item(id <= name.length(), label <= name) => :Item

names <= @["alpha", "beta", "gamma"]

Map[names, _ name = makeItem(name)]() ]=> items
stdout(items.first().unmold().label)
stdout(items.first().unmold().id.toString())
stdout(items.last().unmold().label)
stdout(items.last().unmold().id.toString())

// Expected: alpha
// Expected: 5
// Expected: gamma
// Expected: 5
