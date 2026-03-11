// wasm-edge should reject socket APIs
conn <= tcpConnect("127.0.0.1", 8080)
stdout("should not reach here")
