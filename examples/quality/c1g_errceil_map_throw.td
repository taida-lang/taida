// Quality Test: C-1g — エラー天井内で Map + コールバック内から throw → 捕捉（B-4b + B-5c 複合）
// Map コールバック内の throw が外側 |== に伝播するかを検証
Error => BadVal = @(code: Int)

safeDouble x =
  | x < 0 |> BadVal(type <= "BadVal", message <= "negative", code <= 1).throw()
  | _ |> x * 2
=> :Int

processOk items =
  |== err: Error =
    "caught:" + err.message
  => :Str
  Map[items, _ x = safeDouble(x)]() ]=> result
  Join[result, ","]() ]=> joined
  joined
=> :Str

processFail items =
  |== err: Error =
    "caught:" + err.message
  => :Str
  Map[items, _ x = safeDouble(x)]() ]=> result
  Join[result, ","]() ]=> joined
  joined
=> :Str

stdout(processOk(@[1, 2, 3]))
stdout(processFail(@[1, -1, 3]))
// Expected:
// 2,4,6
// caught:negative
