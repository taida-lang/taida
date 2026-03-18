// B-2a: ラムダ内関数定義
// ラムダ内でローカル関数を定義して呼び出し

makeAdder x =
  add y = y + x => :Int
  add
=> :Int

adder <= makeAdder(5)
stdout(adder(10).toString())

// Expected: 15
