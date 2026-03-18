// wasm-wasi acceptance: writeFile failure shape validation
// Verifies that the Result toString shows "throw" is set (not empty Result)
// This file is NOT included in parity tests (wasm-wasi specific)

result <= writeFile("/nonexistent_dir_xyz/file.txt", "data")
stdout(result.isError().toString())
stdout(result.toString())
