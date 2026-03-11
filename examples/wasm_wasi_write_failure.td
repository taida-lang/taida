// wasm-wasi acceptance: writeFile failure path
// writeFile to non-existent directory returns a failed Result

result <= writeFile("/nonexistent_dir_xyz/file.txt", "data")
stdout(result.isError().toString())
