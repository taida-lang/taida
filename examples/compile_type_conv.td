// compile_type_conv.td -- Native type conversion molds test
// Tests: Str[x](), Int[x](), Float[x](), Bool[x]()
// All return Lax[T], unmold with ]=>

// ── Str[x]() ─────────────────────────────────────────────
// Test 1: Int to Str
Str[42]() ]=> s1
stdout(s1)

// Test 2: Bool to Str (true)
Str[true]() ]=> s2
stdout(s2)

// Test 3: Bool to Str (false)
Str[false]() ]=> s3
stdout(s3)

// Test 4: Str to Str (identity)
Str["hello"]() ]=> s4
stdout(s4)

// ── Int[x]() ─────────────────────────────────────────────
// Test 5: Int to Int (identity)
Int[99]() ]=> i1
debug(i1)

// Test 6: Str to Int (valid parse)
Int["123"]() ]=> i2
debug(i2)

// Test 7: Str to Int (invalid parse — returns default 0)
Int["abc"]() ]=> i3
debug(i3)

// Test 8: Bool to Int (true -> 1)
Int[true]() ]=> i4
debug(i4)

// Test 9: Bool to Int (false -> 0)
Int[false]() ]=> i5
debug(i5)

// ── Bool[x]() ────────────────────────────────────────────
// Test 10: Int to Bool (nonzero -> true)
Bool[1]() ]=> b1
debug(b1)

// Test 11: Int to Bool (zero -> false)
Bool[0]() ]=> b2
debug(b2)

// Test 12: Str to Bool ("true")
Bool["true"]() ]=> b3
debug(b3)

// Test 13: Str to Bool ("false")
Bool["false"]() ]=> b4
debug(b4)

// Test 14: Str to Bool (invalid — returns default false)
Bool["maybe"]() ]=> b5
debug(b5)

// Test 15: hasValue check on valid conversion
valid <= Int["42"]()
debug(valid.hasValue())

// Test 16: hasValue check on failed conversion
invalid <= Int["not_a_number"]()
debug(invalid.hasValue())
