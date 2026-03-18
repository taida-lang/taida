// B-1i: 関数内パイプライン — data => Mold[...]() => result
processNums nums =
  Filter[nums, _ x = Mod[x, 2]().unmold() == 0]() ]=> evens
  Map[evens, _ x = x * 10]() ]=> scaled
  Sum[scaled]() ]=> total
  total
=> :Int

data <= @[1, 2, 3, 4, 5, 6]
stdout(processNums(data).toString())
// Expected: 120
