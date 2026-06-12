// F62B-023 (b): list type aliases — `Name = @[ElemType]` names a list type.
// The alias is checker-only: it expands at every annotation site and has no
// runtime artifact. `@(` after `=` stays the pack type-def path.

Pairs = @[@(name: Str, value: Str)]

// The headline idiom: an empty literal pinned by an alias annotation.
emptyPairs: Pairs <= @[]
stdout(emptyPairs.length().toString())

pairs: Pairs <= @[@(name <= "k", value <= "v")]
stdout(pairs.length().toString())
first <=< pairs.first()
stdout(first.name + "=" + first.value)

// Aliases work in parameter and return annotations.
countPairs ps: Pairs = ps.length() => :Int
stdout(countPairs(pairs).toString())

// Alias chains expand at registration: Grid = @[@[Int]].
Row = @[Int]
Grid = @[Row]
g: Grid <= @[@[1, 2], @[3]]
stdout(g.length().toString())
