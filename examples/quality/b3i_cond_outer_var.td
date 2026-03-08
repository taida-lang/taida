// B-3i: Access outer function variable from pattern match arm

greet lang name =
  prefix <= "Hello"
  | lang == "ja" |> prefix + " " + name + "-san"
  | lang == "en" |> prefix + " " + name
  | _ |> prefix + "!"
=> :Str

stdout(greet("ja", "Taida"))
stdout(greet("en", "World"))
stdout(greet("fr", ""))

// Expected:
// Hello Taida-san
// Hello World
// Hello!
