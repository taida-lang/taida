// Quality Test: B-5h — HOF 内パイプライン
// コールバック内でネストした HOF (Fold) を呼び出す

sumList list =
  Fold[list, 0, _ acc x = acc + x]() ]=> total
  total
=> :Int

items <= @[@[1, 2, 3], @[4, 5, 6], @[7, 8, 9]]

Map[items, _ list = sumList(list)]() ]=> sums
stdout(sums)

// Expected: @[6, 15, 24]
