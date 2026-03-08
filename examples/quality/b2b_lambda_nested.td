// B-2b: ラムダ内ラムダ（多段ネスト）
// 3段ラムダ + 各段から外側変数を参照

a <= 1
f <= _ x = _ y = _ z = z + y + x + a

g <= f(2)
h <= g(10)
result <= h(100)
stdout(result.toString())

// f(2) returns _ y = _ z = z + y + 2 + 1
// g(10) returns _ z = z + 10 + 2 + 1
// h(100) returns 100 + 10 + 2 + 1 = 113
// Expected: 113
