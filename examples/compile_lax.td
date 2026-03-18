// Lax[T] basic tests for native compiler

// Test 1: Div returns Lax, unmold extracts value
Div[100, 4]() ]=> d
debug(d)

// Test 2: Mod returns Lax, unmold extracts value
Mod[17, 5]() ]=> m
debug(m)

// Test 3: Division by zero returns empty Lax (unmold gives default 0)
Div[42, 0]() ]=> z
debug(z)

// Test 4: list.get returns Lax
nums <= @[10, 20, 30]
nums.get(1) ]=> val
debug(val)

// Test 5: list.get OOB returns empty Lax
nums.get(99) ]=> oob
debug(oob)

// Test 6: list.first returns Lax
nums.first() ]=> f
debug(f)

// Test 7: list.last returns Lax
nums.last() ]=> l
debug(l)

// Test 8: Lax hasValue check
result <= Div[10, 2]()
debug(result.hasValue())

// Test 9: Lax getOrDefault
Div[10, 0]().getOrDefault(999) ]=> fallback
debug(fallback)

// Test 10: Lax[value]() constructor
Lax[42]() ]=> lval
debug(lval)
