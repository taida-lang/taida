// Quality Test: C-2b — BuchiPack 5 段ネスト — 生成・アクセス・jsonEncode
deep <= @(a <= @(b <= @(c <= @(d <= @(e <= 1)))))
stdout(deep.a.b.c.d.e.toString())
stdout(jsonEncode(deep))
// Expected:
// 1
// {"a":{"b":{"c":{"d":{"e":1}}}}}
