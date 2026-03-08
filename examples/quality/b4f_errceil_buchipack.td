// Quality Test: B-4f — エラー天井内 BuchiPack 生成
// |== 本体内で BuchiPack を生成して返す

Error => MyError = @(code: Int)

build x =
  |== error: Error =
    @(status <= "error", value <= 0)
  => :@(status: Str, value: Int)
  | x < 0 |> MyError(type <= "MyError", message <= "bad", code <= 1).throw()
  | _ |> @(status <= "ok", value <= x)
=> :@(status: Str, value: Int)

r1 <= build(42)
r2 <= build(-1)
stdout(r1.status + ":" + r1.value.toString())
stdout(r2.status + ":" + r2.value.toString())

// Expected:
// ok:42
// error:0
