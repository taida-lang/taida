// Helper module: export function that depends on a sibling value
base <= 3

add x =
  x + base
=> :Int

<<< @(add, base)
