// Prelude functions -- native compile test
// Tests jsonEncode, jsonPretty (stdin excluded: requires interactive input)

// Define a schema
Pilot = @(name: Str, age: Int, active: Bool)

// Create a BuchiPack directly (no __type in native, fields in creation order)
pilot <= Pilot(name <= "Asuka", age <= 14, active <= true)

// jsonEncode: compact JSON string
json <= jsonEncode(pilot)
stdout("Encoded: " + json)

// jsonPretty: pretty-printed JSON string
pretty <= jsonPretty(pilot)
stdout("Pretty:")
stdout(pretty)

// jsonEncode with nested BuchiPack
Address = @(city: Str, zip: Str)
Person = @(name: Str, address: Address)
person <= Person(name <= "Rei", address <= Address(city <= "Tokyo-3", zip <= "999"))
nestedJson <= jsonEncode(person)
stdout("Nested: " + nestedJson)

// jsonEncode with list
numbers <= @[1, 2, 3]
listJson <= jsonEncode(numbers)
stdout("List: " + listJson)

// jsonEncode with integer
intJson <= jsonEncode(42)
stdout("Int: " + intJson)

// jsonEncode with string
strJson <= jsonEncode("hello")
stdout("Str: " + strJson)
