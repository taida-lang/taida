// wasm-edge should reject process APIs
result <= execShell("echo hi")
stdout("should not reach here")
