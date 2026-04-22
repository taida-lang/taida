// C23B-003 reopen + C24-A guard: `Str[Gorillax[v]()]()` must return the
// interpreter's full-form `@(hasValue <= true, __value <= v, __error <=
// @(), __type <= "Gorillax")`. Pre-fix, JS rendered method source bodies;
// native's `taida_pack_to_display_string_full` skipped every field because
// `__error` wasn't in the name registry (printed `@()`). WASM used a
// different hash (`isOk` vs `hasValue`) for Gorillax construction, which
// C23 skipped as a known exception. C24-A (2026-04-23) unified WASM to
// use `hasValue` (matching interpreter / JS / native) and registered
// `__error` in the field-name table. This fixture is now 4-backend
// (interpreter / JS / native / wasm-wasi).
g <= Gorillax[42]()
stdout(Str[g]())
