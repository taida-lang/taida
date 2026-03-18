// RC-1a: Pipeline Patterns and Data Flow Edge Cases
// Verifies pipeline semantics, _ placeholder, and complex flows

// === 1. Forward pipeline with _ placeholder ===

// 1a: String pipeline
"  hello world  " => Trim[_]() => Upper[_]() => result1
stdout(result1)

// 1b: Numeric pipeline
Abs[-42]() => Clamp[_, 0, 100]() => result2
stdout(result2.toString())

// 1c: List pipeline
@[5, 2, 8, 1, 9, 3, 7] => Filter[_, _ x = x > 3]() => Sort[_]() => Take[_, 3]() => result3
stdout(result3)

// 1d: Pipeline into variable
@[1, 2, 3, 4, 5] => Filter[_, _ n = n > 2]() => Map[_, _ n = n * 10]() => result4
stdout(result4)

// === 2. Complex mold pipelines ===

// 2a: Split then Join
Split["a-b-c", "-"]() => Join[_, ","]() => result5
stdout(result5)

// 2b: Map then Sum
@[1, 2, 3] => Map[_, _ x = x * x]() => Sum[_]() => result6
stdout(result6.toString())

// === 3. Intermediate variables (no pipeline) ===

// 3a: Step by step
numbers <= @[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
Filter[numbers, _ x = x > 5]() ]=> step1
Map[step1, _ x = x * 2]() ]=> step2
Fold[step2, 0, _ acc x = acc + x]() ]=> step3
stdout(step3.toString())

// === 4. Condition as pipeline step result ===

classify x: Int =
  | x > 100 |> "high"
  | x > 50 |> "mid"
  | _ |> "low"
=> :Str

stdout(classify(150))
stdout(classify(75))
stdout(classify(10))

// === 5. Pipeline with function calls ===

double x: Int =
  x * 2
=> :Int

triple x: Int =
  x * 3
=> :Int

5 => double(_) => triple(_) => result7
stdout(result7.toString())

// === 6. Complex data processing pattern ===

staff <= @[
  @(name <= "Asuka", age <= 14, role <= "pilot", active <= true),
  @(name <= "Shinji", age <= 14, role <= "pilot", active <= true),
  @(name <= "Rei", age <= 14, role <= "pilot", active <= false),
  @(name <= "Ritsuko", age <= 30, role <= "scientist", active <= true)
]

// Active names
staff => Filter[_, _ s = s.active]() => Map[_, _ s = s.name]() => activeNames
stdout(activeNames)

// Age statistics
staff => Map[_, _ s = s.age]() => ages
Sum[ages]() ]=> totalAge
Div[totalAge, ages.length()]() ]=> avgAge
stdout(avgAge.toString())

// === 7. Nested condition in pipeline context ===

data <= @[1, 2, 3, 4, 5]
classify7 x: Int =
  | x > 3 |> x * 10
  | _ |> x
=> :Int
Map[data, classify7]() ]=> processed
stdout(processed)

// === 8. Pipeline with Lax handling ===

@[10, 20, 30] => items8
items8.get(1) ]=> val8
stdout(val8.toString())

items8.get(100).getOrDefault(99) => fallback8
stdout(fallback8.toString())

// === 9. Partial application in pipeline ===

add x: Int y: Int =
  x + y
=> :Int

add5 <= add(5, )
stdout(add5(3).toString())

multiply x: Int y: Int =
  x * y
=> :Int

double2 <= multiply(, 2)
stdout(double2(7).toString())

// === 10. Deep pipeline chains ===
// NOTE: Multi-line pipelines with indentation DO work in some contexts.
// Using single-line to be safe.
@[1, 2, 3, 4, 5, 6, 7, 8, 9, 10] => Filter[_, _ x = x > 2]() => Filter[_, _ x = x < 8]() => Map[_, _ x = x * 2]() => Sort[_](reverse <= true) => Take[_, 3]() => result10
stdout(result10)

stdout("=== ALL PIPELINE TESTS PASSED ===")
