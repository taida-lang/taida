// Large-int payload through real unmold machinery (Lax tag/value slots
// must not misread a >2^32 payload as a heap pointer). The original
// bare-operand form (`x >=> y` on a raw Int) is rejected by [E1545] now;
// the raw-operand pointer-heuristic edges stay pinned (unchecked) in
// tests/f62b026_unmold_gorilla.rs.
Lax[1234567890123]() >=> y
stdout(y.toString())
