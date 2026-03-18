// B-8g helper: export TypeDef directly (QF-17 fix)
Dog = @(
  name: Str
  age: Int
  bark =
    "Woof:" + name
  => :Str
)

<<< @(Dog)
