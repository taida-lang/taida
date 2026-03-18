// wasm-wasi acceptance: Read + writeFile + Exists
// Run: wasmtime run --dir=. output.wasm

writeFile("_wasi_test_tmp.txt", "hello from wasi")
content <= Read["_wasi_test_tmp.txt"]()
content ]=> text
stdout(text)
