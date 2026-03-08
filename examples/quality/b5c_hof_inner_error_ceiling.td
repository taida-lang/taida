// Quality Test: B-5c — HOF 内エラー天井
// Map のコールバック内で |== を使用

Error => DivErr = @(message: Str)

safeInvert x =
  |== err: Error = 0 - 1 => :Int
  | x == 0 |> DivErr(type <= "DivErr", message <= "zero").throw()
  | _ |> Div[100, x]().unmold()
=> :Int

items <= @[10, 20, 0, 40]

Map[items, _ x = safeInvert(x)]() ]=> result
stdout(result)

// Expected: @[10, 5, -1, 2]
