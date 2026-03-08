// Quality Test: B-6e — BuchiPack 内 BuchiPack（深層ネスト）
// @(a <= @(b <= @(c <= 1))) の生成・参照・jsonEncode

data <= @(
  a <= @(
    b <= @(
      c <= 42
    )
  )
)

stdout(data.a.b.c.toString())
stdout(jsonEncode(data))
// Expected: 42
// Expected: {"a":{"b":{"c":42}}}
