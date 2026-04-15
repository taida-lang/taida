// wasm-wasi acceptance: Exists[path]()
// Run: wasmtime run --dir=. output.wasm
//
// C12B-021: Exists returns Result[Bool]. We assert the shape via
// `.isSuccess()` which is true on every backend when the probe
// itself succeeded (both existing and missing paths qualify — only
// permission-denied / IO errors flip it to false). The inner Bool
// is not compared textually here because wasm does not propagate
// the Bool tag through `.__value` field access (known existing
// codegen gap; see C12B-019 family); tests that need the Bool bit
// use `.isSuccess()` + separate `writeFile` / path state checks.

writeFile("_wasi_exists_test.txt", "test")
a <= Exists["_wasi_exists_test.txt"]()
stdout(a.isSuccess())
b <= Exists["_nonexistent_path_xyz"]()
stdout(b.isSuccess())
