// C19B-001 / C19B-002: 3-backend failure-path parity for
// `runInteractive` when the target program does not exist (ENOENT).
//
// Pins the IoError contract on the failure path:
// - hasValue = false
// - __error.type   = "IoError"      (not ProcessError)
// - __error.kind   = "not_found"    (classify_io_error_kind for ENOENT)
// - __error.code   = 2              (POSIX ENOENT)
//
// Why this matters:
//   Before C19B-001 the Native backend collapsed execvp failure into a
//   child `_exit(127)` and the parent surfaced that as a generic
//   ProcessError(127) — indistinguishable from a program that merely
//   exited with 127. The errno-pipe fix lets Native push the child's
//   errno to the parent and build a proper IoError, matching Interpreter
//   and JS. Before C19B-002 the Native Gorillax field-hash mismatch made
//   `.__error.<field>` silently unreachable; the HASH___ERROR fix unblocks
//   this parity test.

r <= runInteractive("/nonexistent/taida_c19_xyz_does_not_exist", @[])
stdout("hasValue=" + r.hasValue.toString())
stdout("type=" + r.__error.type.toString())
stdout("kind=" + r.__error.kind.toString())
stdout("code=" + r.__error.code.toString())
