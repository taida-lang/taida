// F62B-020: Lte / Between ordering molds. `<=` is the binding operator, so
// "less than or equal" is a mold; Between is the closed-interval range
// check. Operand rules mirror `<` / `>` (numeric pairs, Str pairs by code
// point order, same-Enum pairs by declaration order).

stdout(Lte[1, 2]().toString())
stdout(Lte[2, 2]().toString())
stdout(Lte[3, 2]().toString())

stdout(Lte["a", "b"]().toString())
stdout(Lte["b", "b"]().toString())
stdout(Lte["c", "b"]().toString())

stdout(Lte[1.5, 2.5]().toString())
stdout(Lte[2.5, 1.5]().toString())

stdout(Between[5, 1, 10]().toString())
stdout(Between[1, 1, 10]().toString())
stdout(Between[10, 1, 10]().toString())
stdout(Between[0, 1, 10]().toString())
stdout(Between[11, 1, 10]().toString())

// the original pain point: character range checks
c <= "k"
stdout(Between[c, "a", "z"]().toString())
stdout(Between["A", "a", "z"]().toString())

// pipe form
7 => Between[_, 1, 10]() => inRange
stdout(inRange.toString())
