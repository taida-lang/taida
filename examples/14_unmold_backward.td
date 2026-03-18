// === Taida Lang: <=[ (UnmoldBackward) ===

// Basic unmold backward
lax <= Lax[42]()
value <=[ lax
stdout(`Unmold backward: ${value}`)

// With string
strLax <= Lax["hello"]()
strVal <=[ strLax
stdout(`String unmold: ${strVal}`)

// Async unmold backward
asyncVal <= Async[100]()
result <=[ asyncVal
stdout(`Async unmold backward: ${result}`)

// Map then unmold backward
numbers <= @[1, 2, 3, 4, 5]
doubled <=[ Map[numbers, _ x = x * 2]()
stdout(`Map with <=[: ${doubled}`)

// Filter then unmold backward
isEven x =
  Mod[x, 2]().unmold() == 0
=> :Bool
evens <=[ Filter[numbers, isEven]()
stdout(`Filter with <=[: ${evens}`)

// Reduce then unmold backward
total <=[ Reduce[numbers, 0, _ acc x = acc + x]()
stdout(`Reduce with <=[: ${total}`)
