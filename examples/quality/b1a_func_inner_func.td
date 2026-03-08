// B-1a: 関数内関数定義 — ローカル関数を定義して呼び出し
outer x =
  inner y = y * 3 => :Int
  inner(x) + inner(x + 1)
=> :Int

stdout(outer(5).toString())
// Expected: 33
