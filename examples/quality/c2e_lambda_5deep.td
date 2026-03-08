// Quality Test: C-2e — ラムダ 5 段ネスト — 5 段クロージャで各段から変数 capture
a <= 1
f1 <= _ = _ = _ = _ = _ = a + a + a + a + a
stdout(f1()()()()().toString())
// Expected: 5
