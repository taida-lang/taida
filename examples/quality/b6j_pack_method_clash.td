// Quality Test: B-6j — BuchiPack 組み込みメソッド名衝突
// length, isEmpty, toString, keys をフィールド名に使用（F-57/58 回帰）

obj <= @(
  length <= 42,
  isEmpty <= false,
  keys <= "myKeys"
)

stdout(obj.length.toString())
stdout(obj.isEmpty.toString())
stdout(obj.keys)
// Expected: 42
// Expected: false
// Expected: myKeys
