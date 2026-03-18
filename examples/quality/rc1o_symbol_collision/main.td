// RC-1o: Symbol collision test — main entry point
// Imports compute from both mod_a and mod_b (aliased).
// Both modules have identically-named _helper and compute functions.
// module_key namespacing must prevent collision.
// Expected:
//   computeA(5) = _helper_a(5) + 1 = 10 + 1 = 11
//   computeB(5) = _helper_b(5) + 2 = 15 + 2 = 17

>>> ./mod_a.td => @(compute => computeA)
>>> ./mod_b.td => @(compute => computeB)

stdout(computeA(5).toString())
stdout(computeB(5).toString())
