// Quality Test: B-9l — ErrorCeiling 脱出時のリソース解放
// throw 時に途中の BuchiPack/List が正しく解放されるか

Error => DataError = @(code: Int)

process items =
  |== error: Error =
    "error:" + error.message
  => :Str
  | items.length() == 0 |> DataError(type <= "DataError", message <= "empty", code <= 1).throw()
  | _ |>
    Map[items, _ x = x * 2]() ]=> doubled
    Fold[doubled, 0, _ acc x = acc + x]() ]=> total
    "total:" + total.toString()
=> :Str

stdout(process(@[1, 2, 3]))
stdout(process(@[]))
stdout(process(@[10, 20]))
// Expected:
// total:12
// error:empty
// total:60
