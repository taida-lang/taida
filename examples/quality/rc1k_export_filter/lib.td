// RC-1k: Export filter test — library module
// This module exports `public_fn` but NOT `_private_helper`.
// The WASM inliner must NOT include `_private_helper` in the main module
// unless it is transitively needed by `public_fn`.

_private_helper x: Int =
  x * 100
=> :Int

_another_private y: Str =
  "secret: " + y
=> :Str

public_fn x: Int =
  x + 1
=> :Int

public_uses_private x: Int =
  _private_helper(x) + 1
=> :Int

<<< @(public_fn, public_uses_private)
