// RC-1n: Diamond dependency test — main entry point
// Diamond pattern: main -> B, main -> C, B -> D, C -> D
// D should be resolved once (no duplicate inlining).
// Internal functions (_b_helper, _c_helper, _secret) should not collide.
// Expected output:
//   B:D:[x]
//   C:D:[y]

>>> ./mod_b.td => @(fromB)
>>> ./mod_c.td => @(fromC)

stdout(fromB("x"))
stdout(fromC("y"))
