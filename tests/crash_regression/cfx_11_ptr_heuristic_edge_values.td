// Pointer-heuristic edge payloads (page-boundary 4097, >2^32) through
// real unmold machinery. See cfx_r2_unmold_large_int.td for why the
// bare-operand form moved to the f62b026 suite.
Lax[4097]() >=> y1
Lax[1234567890123]() >=> y2
flat <= Flatten[@[65537]]()
stdout(y1.toString())
stdout(y2.toString())
stdout(flat)
