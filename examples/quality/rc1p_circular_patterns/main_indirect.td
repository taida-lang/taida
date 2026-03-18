// RC-1p: Indirect circular test (A -> B -> C -> A)
// Expected: Circular import detected error
>>> ./indirect_a.td => @(fromA)
stdout(fromA("test"))
