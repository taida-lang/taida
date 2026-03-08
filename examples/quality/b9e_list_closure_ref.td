// Quality Test: B-9e — List 内 closure が List 自身を参照
// items を参照する closure を作るパターン
// 不変値のスナップショットにより循環しない

items <= @[10, 20, 30]
stdout(items.length().toString())

// lambda が items をキャプチャして参照
sumItems =
  Fold[items, 0, _ acc x = acc + x]() ]=> s
  s
=> :Int
stdout(sumItems().toString())

// items を使う関数の結果をリストに格納
lengths <= @[items.length(), items.length(), items.length()]
stdout(lengths)
// Expected:
// 3
// 60
// @[3, 3, 3]
