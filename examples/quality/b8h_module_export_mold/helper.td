// B-8h helper: export function that returns mold result
safeDivide x: Int y: Int =
  Div[x, y]() >=> result
  result
=> :Int

<<< @(safeDivide)
