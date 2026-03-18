// Function overloading is disallowed — use distinct names instead.
// This test verifies that two functions with different names and signatures
// work correctly (overloading would be caught by taida check).

createProfileFromPack param: @(name: Str, age: Int) =
  @(name <= param.name, age <= param.age, active <= true)
=> :@(name: Str, age: Int, active: Bool)

createProfileFromArgs name: Str age: Int =
  @(name <= name, age <= age, active <= true)
=> :@(name: Str, age: Int, active: Bool)

profile1 <= createProfileFromPack(@(name <= "Rei", age <= 14))
stdout(profile1.name)

profile2 <= createProfileFromArgs("Asuka", 14)
stdout(profile2.name)
