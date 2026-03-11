// wasm-edge env test: environment variables via taida_host imports
env_a <= EnvVar["TAIDA_TEST_A"]()
env_a ]=> val
stdout(val)

all <= allEnv()
stdout(all.size().toString())
