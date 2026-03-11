// W-5: Error ceiling test — error handling with |==
validate text =
  |== error: Error =
    | _ |> "caught: " + error.message
  => :Str
  | text == "" |> Error(type <= "ValidationError", message <= "Empty input").throw()
  | _ |> "Valid: " + text
=> :Str

stdout(validate("hello"))
stdout(validate(""))
