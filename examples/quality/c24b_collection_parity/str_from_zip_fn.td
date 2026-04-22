// C25B-027 guard: function-form `zip(a, b)` must match the mold-form
// `Zip[a, b]()` byte-for-byte across all four backends. Pre-fix, the
// mold form was wired to `taida_list_zip` by `src/codegen/lower_molds.rs`
// but the function form fell through `stdlib_runtime_funcs` lookup →
// user-func lookup → lambda `CallIndirect` → native segfault (139) /
// wasm `uninitialized element` trap. C25B-027 (2026-04-23, Codex reopen
// of C24-B HOLD) adds `zip` / `enumerate` to `stdlib_runtime_funcs` so
// the function form routes to the same runtime helper. Interpreter is
// the source of truth (`src/interpreter/prelude.rs::zip`).
a <= @[1, 2, 3]
b <= @["x", "y", "z"]
stdout(Str[zip(a, b)]())
