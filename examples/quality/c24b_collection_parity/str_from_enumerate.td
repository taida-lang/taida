// C24-B guard: `Str[Enumerate[xs]()]()` must match the interpreter's
// `@[@(index <= 0, value <= …), …]` shape byte-for-byte across all four
// backends. Symmetric with `str_from_zip.td` — the `index` / `value`
// field names were hashed but not name-registered, so native /
// wasm pair packs rendered as `@()` (and segfaulted / trapped on
// recursion). Interpreter + JS were already correct.
xs <= @["a", "b", "c"]
stdout(Str[Enumerate[xs]()]())
