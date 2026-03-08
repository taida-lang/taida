// Quality Test: C-1f — TypeDef メソッド内で HashMap.get → Lax unmold → パターンマッチ
Registry = @(
  data: HashMap[Str, Int]
  lookup key =
    lax <= data.get(key)
    | lax.hasValue() |> lax.getOrDefault(0).toString()
    | _ |> "not_found"
  => :Str
)

m <= hashMap().set("a", 10).set("b", 20)
reg <= Registry(data <= m)
stdout(reg.lookup("a"))
stdout(reg.lookup("b"))
stdout(reg.lookup("z"))
// Expected:
// 10
// 20
// not_found
