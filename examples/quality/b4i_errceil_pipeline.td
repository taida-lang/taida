// Quality Test: B-4i — エラー天井内パイプライン
// |== 内でモールドチェーン + 途中 throw

Error => MyError = @(code: Int)

sumList nums =
  Sum[nums]() ]=> total
  total
=> :Int

transform nums =
  |== error: Error =
    "err:" + error.message
  => :Str
  | nums.length() == 0 |> MyError(type <= "MyError", message <= "empty", code <= 1).throw()
  | _ |> "sum:" + sumList(nums).toString()
=> :Str

stdout(transform(@[1, 2, 3]))
stdout(transform(@[]))

// Expected:
// sum:6
// err:empty
