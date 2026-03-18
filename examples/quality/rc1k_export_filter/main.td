// RC-1k: Export filter test — main module
// Imports only `public_fn` from lib.td.
// The WASM inliner should NOT include `_another_private` from lib.td.
// `_private_helper` should also NOT be included since `public_fn` does not use it.

>>> ./lib.td => @(public_fn)

result <= public_fn(41)
stdout(result.toString())
