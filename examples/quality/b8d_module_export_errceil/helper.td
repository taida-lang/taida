// B-8d helper: export function with error ceiling
Error => BadInput = @(message: Str)

safeParse input =
  |== error: Error =
    "error:" + error.message
  => :Str
  | input == "" |> BadInput(type <= "BadInput", message <= "empty").throw()
  | _ |> "ok:" + input
=> :Str

<<< @(safeParse)
