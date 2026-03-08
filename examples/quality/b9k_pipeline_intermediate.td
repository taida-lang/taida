// Quality Test: B-9k — Pipeline 中間値の lifetime
// Map/Filter のチェーンで中間値が正しく管理されるか

nums <= @[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

isEven x =
  Mod[x, 2]().unmold() == 0
=> :Bool

// パイプラインチェーン
Filter[nums, isEven]() ]=> evens
Map[evens, _ x = x * 10]() ]=> tens
Fold[tens, 0, _ acc x = acc + x]() ]=> total
stdout(total.toString())

// 元のリストは無傷
stdout(nums.length().toString())

// 中間値も参照可能
stdout(evens)
stdout(tens)
// Expected:
// 300
// 10
// @[2, 4, 6, 8, 10]
// @[20, 40, 60, 80, 100]
