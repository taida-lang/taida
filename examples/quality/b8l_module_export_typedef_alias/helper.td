// B-8l helper: export TypeDef for alias import
Dog = @(
  name: Str
  bark =
    "Woof:" + name
  => :Str
)

<<< @(Dog)
