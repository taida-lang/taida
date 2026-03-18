// wasm-wasi acceptance: EnvVar + allEnv
// Run: wasmtime run --env TAIDA_TEST_A=hello --env TAIDA_TEST_B=world output.wasm

env_a <= EnvVar["TAIDA_TEST_A"]()
env_a ]=> val
stdout(val)

all <= allEnv()
stdout(all.size().toString())
