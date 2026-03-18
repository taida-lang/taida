// Quality Test: D-3a — 空値の全操作
// 空文字列/空リスト/空 BuchiPack に対する主要モールド・メソッドのテスト

// === 空文字列 ===
s <= ""
stdout(s.length().toString())
stdout(s.contains("x").toString())
u <= Upper[s]()
stdout(u)
lo <= Lower[s]()
stdout(lo)
t <= Trim[s]()
stdout(t)
sp <= Split[s, ","]()
stdout(sp.length().toString())
r <= Reverse[s]()
stdout(r)
stdout(s.startsWith("").toString())
stdout(s.endsWith("").toString())
stdout(s.indexOf("x").toString())

// === 空リスト ===
nonempty <= @[1]
l <= Drop[nonempty, 1]()
stdout(l.length().toString())
stdout(l.isEmpty().toString())
first <= l.first()
stdout(first.hasValue().toString())
last <= l.last()
stdout(last.hasValue().toString())
s2 <= Sum[l]()
stdout(s2.toString())
sorted <= Sort[l]()
stdout(sorted.length().toString())
flat <= Flatten[l]()
stdout(flat.length().toString())
Map[l, _ x = x * 2]() ]=> mapped
stdout(mapped.length().toString())
Filter[l, _ x = x > 0]() ]=> filtered
stdout(filtered.length().toString())
Fold[l, 0, _ acc x = acc + x]() ]=> folded
stdout(folded.toString())

// === 空 BuchiPack ===
p <= @()
stdout(jsonEncode(p))
