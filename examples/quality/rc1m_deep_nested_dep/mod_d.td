// RC-1m: Deep nested dependency — leaf module D
// Provides a string-wrapping function (uses strings to avoid WASM int/ptr heuristic)

_internal_d x = "D(" + x + ")" => :Str

wrap_d x = _internal_d(x) => :Str

<<< @(wrap_d)
