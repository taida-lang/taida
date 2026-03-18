// RC-1k: Export filter test — transitive dependency
// Imports `public_uses_private` which internally calls `_private_helper`.
// The WASM inliner should include `_private_helper` (needed transitively)
// but NOT `_another_private` (not needed by anything imported).

>>> ./lib.td => @(public_uses_private)

result <= public_uses_private(5)
stdout(result.toString())
