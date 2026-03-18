// Quality Test: B-9b — BuchiPack 相互参照
// a.ref = b, b.ref = a は不変セマンティクスで不可能
// 代入時点のスナップショットが格納される

a <= @(name <= "A", val <= 1)
b <= @(name <= "B", ref <= a)
c <= @(name <= "A2", ref <= b)
stdout(b.ref.name)
stdout(b.ref.val.toString())
stdout(c.ref.name)
stdout(c.ref.ref.name)
// Expected:
// A
// 1
// B
// A
