// B-8m helper: export InheritanceDef
Person = @(
  name: Str
  greet =
    "Hi:" + name
  => :Str
)

Person => Employee = @(
  dept: Str
  info =
    name + ":" + dept
  => :Str
)

<<< @(Employee)
