// B-2j: 外部変数 capture 総合
// 関数引数・ローカル変数・ネスト関数の変数を全て capture

prefix <= "["
suffix <= "]"

wrap text = prefix + text + suffix => :Str

outer arg =
  local <= arg + 10
  fn <= _ x = wrap(x.toString()) + ":" + local.toString()
  result <= fn(arg)
  result
=> :Str

stdout(outer(5))

// wrap(5.toString()) = "[5]", local = 15
// Expected: [5]:15
