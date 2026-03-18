// Taida Lang Example: Error Handling with Error Ceiling

// Define custom error types (inheriting from Error)
Error => ValidationError = @(field: Str, code: Int)

// Function with safe division via Div mold
safeDivide x y =
  Div[x, y]() ]=> result
  result
=> :Int

// Normal path
result1 <= safeDivide(10, 2)
stdout(result1)

// Zero division path (returns default 0, no error)
result2 <= safeDivide(10, 0)
stdout(result2)

// Validation with conditional error handler
validate text =
  |== error: Error =
    | error.type == "ValidationError" |> "Invalid: " + error.message
    | _ |> "Unknown error"
  => :Str
  | text == "" |> ValidationError(type <= "ValidationError", message <= "Empty input", field <= "text", code <= 400).throw()
  | _ |> "Valid: " + text
=> :Str

stdout(validate("hello"))
stdout(validate(""))
