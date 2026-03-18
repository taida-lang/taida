// B-8f helper: export nested BuchiPack
makePerson name age =
  @(
    name <= name,
    age <= age,
    address <= @(city <= "Tokyo", zip <= "100")
  )
=> :@(name: Str, age: Int, address: @(city: Str, zip: Str))

<<< @(makePerson)
