// F62B-023 (c): unmold bindings accept a type annotation on the target,
// symmetric in both directions:
//   name: Type <=< expr
//   expr >=> name: Type
// The annotation is checker-only (validated against the unmolded type and
// used as the binding type); runtime behavior matches the untyped form.

half: Int <=< Div[10, 2]()
stdout((half + 1).toString())

Div[20, 4]() >=> fifth: Int
stdout((fifth * 2).toString())

// Untyped forms keep working unchanged.
third <=< Div[9, 3]()
stdout(third.toString())

Div[8, 2]() >=> quarter
stdout(quarter.toString())
