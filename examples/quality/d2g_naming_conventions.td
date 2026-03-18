// D-2g: docs/reference/naming_conventions.md code examples

// PascalCase types
Pilot = @(
  name: Str
  age: Int
)

// snake_case variables
pilot_name <= "Misato"
total_count <= 42
is_active <= true
stdout(pilot_name)
stdout(total_count.toString())
stdout(is_active.toString())

// camelCase functions (doc convention)
getPilotName pilot: Pilot =
  pilot.name
=> :Str

calculateSum items: @[Int] =
  Fold[items, 0, _ acc x = acc + x]() ]=> total
  total
=> :Int

p <= Pilot(name <= "Ritsuko", age <= 30)
stdout(getPilotName(p))
stdout(calculateSum(@[1, 2, 3, 4, 5]).toString())

// UPPER_SNAKE_CASE constants
MAX_RETRY_COUNT <= 3
DEFAULT_TIMEOUT <= 5000
API_BASE_URL <= "https://api.example.com"
stdout(MAX_RETRY_COUNT.toString())
stdout(API_BASE_URL)
