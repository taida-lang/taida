// compile_methods.td — Str/Num/List state check methods test

// ── Str methods ──────────────────────────────────────────
name <= "hello world"
stdout(name.contains("world"))
stdout(name.contains("xyz"))
stdout(name.startsWith("hello"))
stdout(name.startsWith("world"))
stdout(name.endsWith("world"))
stdout(name.endsWith("hello"))
stdout(name.indexOf("world"))
stdout(name.lastIndexOf("o"))
stdout(name.length())
stdout(name.toString())

// Str.get(idx) -> Lax
ch_lax <= name.get(0)
stdout(ch_lax.hasValue())
ch2_lax <= name.get(100)
stdout(ch2_lax.hasValue())

// ── Num methods (Int) ────────────────────────────────────
x <= 42
stdout(x.isPositive())
stdout(x.isNegative())
stdout(x.isZero())
stdout(x.toString())

y <= 0 - 5
stdout(y.isNegative())
stdout(y.isPositive())

z <= 0
stdout(z.isZero())

// Int: isNaN/isInfinite are always false, isFinite is always true
stdout(x.isNaN())
stdout(x.isInfinite())
stdout(x.isFinite())

// ── List methods ─────────────────────────────────────────
nums <= @[10, 20, 30, 20, 40]
stdout(nums.isEmpty())
stdout(nums.contains(20))
stdout(nums.contains(99))
stdout(nums.indexOf(20))
stdout(nums.lastIndexOf(20))

// first/last -> Lax
f <= nums.first()
stdout(f.hasValue())
l <= nums.last()
stdout(l.hasValue())

// get -> Lax
g <= nums.get(2)
stdout(g.hasValue())
g_oob <= nums.get(100)
stdout(g_oob.hasValue())

// max/min -> Lax
mx <= nums.max()
stdout(mx.hasValue())
mn <= nums.min()
stdout(mn.hasValue())

// empty list — use Drop to create
empty <= @[0]
empty_dropped <= Drop[empty, 1]()
stdout(empty_dropped.isEmpty())

// any/all/none with lambda
stdout(nums.any(_ x = x > 25))
stdout(nums.all(_ x = x > 5))
stdout(nums.none(_ x = x > 100))
