// C24-B guard: `Str[Zip[a, b]()]()` must match the interpreter's
// `@[@(first <= …, second <= …), …]` shape byte-for-byte across all four
// backends. Pre-fix, native / WASM registered `HASH_FIRST` / `HASH_SECOND`
// on each pair but never recorded the `first` / `second` field names in
// the runtime's field-name registry, so `taida_pack_to_display_string_full`
// / `_wasm_pack_to_string_full` returned NULL and silently skipped every
// pair field. The outer list then rendered as `@[]` on native (segfault
// on pack-tag recursion) and trapped with "uninitialized element" on
// wasm. Interpreter + JS were already correct.
a <= @[1, 2, 3]
b <= @["x", "y", "z"]
stdout(Str[Zip[a, b]()]())
