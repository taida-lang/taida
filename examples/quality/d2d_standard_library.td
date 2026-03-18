// D-2d: docs/reference/standard_library.md code examples

// Prelude builtins: stdout, stderr, stdin, jsonEncode, jsonPretty
stdout("Hello from prelude")

// jsonEncode
data <= @(name <= "Asuka", age <= 14)
stdout(jsonEncode(data))

// jsonPretty
stdout(jsonPretty(data))

// HashMap - not available in interpreter (QF finding: doc describes it but not implemented)
// Set - not available in interpreter (QF finding: doc describes it but not implemented)
stdout("HashMap/Set: skipped (not implemented)")
