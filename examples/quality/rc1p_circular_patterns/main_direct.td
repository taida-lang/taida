// RC-1p: Direct circular test (A <-> B)
// Expected: Circular import detected error
>>> ./direct_a.td => @(fromA)
stdout(fromA("test"))
