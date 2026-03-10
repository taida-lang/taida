// Negative test: closures are unsupported in wasm-min
makeAdder x =
  _ y = x + y
=> :Int

add5 <= makeAdder(5)
debug(add5(3))
