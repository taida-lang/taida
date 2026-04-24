// C26B-010: Memory smoke fixture — Str primitive + slice teardown.
//
// Purpose: exercise Str allocation + slice view (zero-copy span) drop so
// valgrind catches regressions in the char-index cache + byte-level
// primitive path (C26B-018 Option A+B+C).
//
// Expected stdout (3-backend identical):
//   taida
//   5

s <= "taida"
stdout(s)
stdout(s.length())
