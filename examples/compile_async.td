// Phase N8: Async テスト
// 同期シミュレーション: Async は即座に fulfilled/rejected

// Async 値の生成（fulfilled）
a <= Async[42]()
a ]=> val
debug(val)

// Async 値を関数で使う
doubleAsync x: Int =
  Async[x * 2]()
=> :Async[Int]

result <= doubleAsync(21)
result ]=> doubled
debug(doubled)

// チェーン: Async → unmold → Async → unmold
first <= Async[10]()
first ]=> x
second <= Async[x + 5]()
second ]=> y
debug(y)
