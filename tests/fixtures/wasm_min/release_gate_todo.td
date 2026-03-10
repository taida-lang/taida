// W-2 regression test: --release should reject TODO molds in wasm-min.
TODO["RG-1", "implement this", "later", ""]()
stdout("should not compile with --release")
