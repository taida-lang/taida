// Quality Test: B-10e — Circular import with TypeDef export
// Expects: circular import error, TypeDef resolution should not crash

>>> ./mod_a => @(Person)

p <= Person("Alice", 30)
stdout(p.name)

// Expected: circular import error (not a crash)
