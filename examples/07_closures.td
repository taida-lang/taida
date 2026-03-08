// === Taida Lang: Closures ===

// Function returning a function (closure)
makeAdder x =
  _ y = x + y
=> :Int

add5 <= makeAdder(5)
add10 <= makeAdder(10)

stdout(`add5(3) = ${add5(3)}`)
stdout(`add5(7) = ${add5(7)}`)
stdout(`add10(3) = ${add10(3)}`)
stdout(`add10(20) = ${add10(20)}`)

// Counter-like pattern
makeMultiplier factor =
  _ x = x * factor
=> :Int

double <= makeMultiplier(2)
triple <= makeMultiplier(3)

stdout(`double(7) = ${double(7)}`)
stdout(`triple(7) = ${triple(7)}`)
