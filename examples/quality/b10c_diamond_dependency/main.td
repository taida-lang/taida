// Quality Test: B-10c — Diamond dependency (not circular, should succeed)
// A -> B, A -> C, B -> D, C -> D

>>> ./mod_b => @(fromB)
>>> ./mod_c => @(fromC)

stdout(fromB("x"))
stdout(fromC("y"))

// Expected: B:shared:x
// Expected: C:shared:y
