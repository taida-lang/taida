// Quality Test: B-9g — HashMap value に自身を格納
// hashMap は不変 — set は新しい HashMap を返す
// 自身を格納しようとしても代入時点のスナップショット

m1 <= hashMap().set("a", "hello")
m2 <= m1.set("b", "world")
stdout(m2.get("a").getOrDefault(""))
stdout(m2.get("b").getOrDefault(""))
stdout(m2.size().toString())

// m1 は m2 の影響を受けない（不変）
stdout(m1.size().toString())
stdout(m1.get("b").getOrDefault("missing"))
// Expected:
// hello
// world
// 2
// 1
// missing
