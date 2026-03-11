// wasm-wasi acceptance: Exists[path]()
// Run: wasmtime run --dir=. output.wasm

writeFile("_wasi_exists_test.txt", "test")
stdout(Exists["_wasi_exists_test.txt"]().toString())
stdout(Exists["_nonexistent_path_xyz"]().toString())
