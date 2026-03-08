// B-1b: 関数内ラムダ — ラムダを変数に束縛して呼び出し
// クロージャパターン: 外の変数をキャプチャするラムダを返す
makeDoubler =
  _ x = x * 2
=> :Int

dbl <= makeDoubler()
stdout(dbl(7).toString())
// Expected: 14
