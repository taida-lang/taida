// C26B-010: Memory smoke fixture — minimum-viable interpreter warm-up.
//
// Purpose: provide a <1s end-to-end run that valgrind can wrap without
// exceeding the memory.yml timeout budget. Reproduces the simplest
// stdout + arithmetic path so scope teardown (parser -> type checker ->
// interpreter -> Value drop) is exercised in a single linear program.
//
// Expected stdout (3-backend identical):
//   hello
//   42

stdout("hello")
x <= 40
y <= 2
stdout(x + y)
