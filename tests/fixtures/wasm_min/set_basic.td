// W-4f2: Set basic operations test for wasm-min
// Tests size, add, has, remove, union, intersect, diff, toList AND toString content

s <= setOf(@[1, 2, 3, 2, 1])
stdout("size: " + s.size().toString())

// add / has
s2 <= s.add(4)
stdout("after add size: " + s2.size().toString())
stdout("has 4: " + s2.has(4).toString())

// remove
s3 <= s2.remove(1)
stdout("after remove size: " + s3.size().toString())

// union / intersect / diff
a <= setOf(@[1, 2, 3])
b <= setOf(@[2, 3, 4])
stdout("union size: " + a.union(b).size().toString())
stdout("intersect size: " + a.intersect(b).size().toString())
stdout("diff size: " + a.diff(b).size().toString())

// toList
stdout("toList len: " + s.toList().length().toString())

// W-4f2: toString — verifies full Set formatting
stdout(s.toString())

// W-4f2: union toString
stdout(a.union(b).toString())
