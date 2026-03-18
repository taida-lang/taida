// Quality Test: B-10b — Indirect circular import (A -> B -> C -> A)
// Expects: circular import error detected

>>> ./mod_a => @(start)

stdout(start("test"))

// Expected: circular import error
