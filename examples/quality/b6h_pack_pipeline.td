// Quality Test: B-6h — BuchiPack 内パイプライン（ラムダ経由）
// フィールド関数内でパイプライン

processFn nums =
  Filter[nums, _ x = Mod[x, 2]().unmold() == 0]() ]=> evens
  Sum[evens]() ]=> total
  total
=> :Int

obj <= @(process <= _ nums = processFn(nums))

data <= @[2, 3, 4, 5, 6]
stdout(obj.process(data).toString())
// Expected: 12
