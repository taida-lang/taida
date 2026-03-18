// Quality Test: C-1b — Map 内 Filter 内ラムダで外側変数参照（HOF 多段ネスト + capture）
threshold <= 2
prefix <= "v:"

data <= @[1, 2, 3, 4, 5]
filtered <= Filter[data, _ x = x > threshold]()
filtered ]=> fList
mapped <= Map[fList, _ x = prefix + x.toString()]()
mapped ]=> mList
stdout(mList)
// Expected: @[v:3, v:4, v:5]
