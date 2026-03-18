// Quality Test: C-4a — HOF throw variants: Map/Filter/Fold throw caught by outer |==
Error => NegErr = @(code: Int)

guardMap x =
  | x < 0 |> NegErr(type <= "NegErr", message <= "neg", code <= 1).throw()
  | _ |> x * 10
=> :Int

guardFilter x =
  | x < 0 |> NegErr(type <= "NegErr", message <= "neg", code <= 1).throw()
  | _ |> true
=> :Bool

guardFold acc x =
  | x < 0 |> NegErr(type <= "NegErr", message <= "neg", code <= 1).throw()
  | _ |> acc + x
=> :Int

safeMap items =
  |== err: Error =
    "map:" + err.message
  => :Str
  Map[items, _ x = guardMap(x)]() ]=> r
  Join[r, ","]() ]=> v
  v
=> :Str

safeFilter items =
  |== err: Error =
    "filter:" + err.message
  => :Str
  Filter[items, _ x = guardFilter(x)]() ]=> r
  Join[r, ","]() ]=> v
  v
=> :Str

safeFold items =
  |== err: Error =
    "fold:" + err.message
  => :Str
  Fold[items, 0, _ acc x = guardFold(acc, x)]() ]=> v
  v.toString()
=> :Str

stdout(safeMap(@[1, -1, 3]))
stdout(safeFilter(@[1, -1, 3]))
stdout(safeFold(@[1, -1, 3]))
// Expected:
// map:neg
// filter:neg
// fold:neg
