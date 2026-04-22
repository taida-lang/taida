// C25B-027 guard: function-form `enumerate(xs)` must match the mold-form
// `Enumerate[xs]()` byte-for-byte across all four backends. Symmetric
// with `str_from_zip_fn.td`. Pre-fix, native segfaulted and wasm trapped
// with `uninitialized element` because the function-form lowering fell
// through to `CallIndirect` on an undefined lambda. C25B-027 routes it
// to `taida_list_enumerate` via `stdlib_runtime_funcs`.
xs <= @["a", "b", "c"]
stdout(Str[enumerate(xs)]())
