// Quality Test: D-3b — 1万件リストの操作
// 1万件のリストを末尾再帰で生成し、Map/Filter/Sort/Flatten を検証

buildList n acc =
  | n == 0 |> acc
  | _ |> buildList(n - 1, Prepend[acc, n]())
=> :@[Int]

data <= buildList(10000, @[])
stdout(data.length().toString())

// Map
Map[data, _ x = x * 2]() ]=> mapped
stdout(mapped.length().toString())
mapped.first() ]=> mf
stdout(mf.toString())
mapped.last() ]=> ml
stdout(ml.toString())

// Filter (even numbers)
isEven x =
  Mod[x, 2]() ]=> r
  r == 0
=> :Bool

Filter[data, isEven]() ]=> filtered
stdout(filtered.length().toString())

// Sort
sorted <= Sort[data]()
sorted.first() ]=> sf
stdout(sf.toString())
sorted.last() ]=> sl
stdout(sl.toString())

// Sum
s <= Sum[data]()
stdout(s.toString())

// Flatten (small nested list - separate from main 10k test)
nested <= @[@[1, 2], @[3, 4], @[5, 6]]
flat <= Flatten[nested]()
stdout(flat.length().toString())
