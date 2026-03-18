// Example 30: Class-like Methods
// Demonstrates method definitions in TypeDef and InheritanceDef

// === Basic type with methods ===

Greeter = @(
  name: Str
  greet =
    "Hello, " + name + "!"
  => :Str
)

g <= Greeter(name <= "Asuka")
g.greet() => greeting
stdout(greeting)

// === Method with arguments ===

Calculator = @(
  base: Int
  add x: Int =
    base + x
  => :Int
  mul x: Int =
    base * x
  => :Int
)

calc <= Calculator(base <= 10)
calc.add(5) => sum
calc.mul(3) => product
stdout(sum.toString())
stdout(product.toString())

// === Inheritance with methods ===

Pilot = @(
  name: Str
  age: Int
  introduce =
    "I'm " + name + ", age " + age.toString()
  => :Str
)

Pilot => EvaPilot = @(
  seq: Int
  callSign =
    name + " #" + seq.toString()
  => :Str
)

asuka <= EvaPilot(name <= "Asuka", age <= 14, seq <= 2)
asuka.introduce() => intro
asuka.callSign() => sign
stdout(intro)
stdout(sign)

// === Method override ===

Shape = @(
  width: Int
  height: Int
  area =
    width * height
  => :Int
)

Shape => Triangle = @(
  area =
    Div[width * height, 2]() ]=> result
    result
  => :Int
)

rect <= Shape(width <= 10, height <= 5)
tri <= Triangle(width <= 10, height <= 5)
stdout(rect.area().toString())
stdout(tri.area().toString())
