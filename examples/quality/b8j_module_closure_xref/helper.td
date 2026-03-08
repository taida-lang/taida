// B-8j helper: closure cross-reference (F-56 regression)
// export function A's closure references non-export function B

formatNum n =
  "[" + n.toString() + "]"
=> :Str

wrapAll items =
  Map[items, _ x = formatNum(x)]() ]=> result
  Join[result, ","]() ]=> joined
  joined
=> :Str

<<< @(wrapAll)
