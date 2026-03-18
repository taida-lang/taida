// B-3g: Mold + unmold inside pattern match arm

safeMath op a b =
  | op == "div" |> Div[a, b]().unmold()
  | op == "mod" |> Mod[a, b]().unmold()
  | _ |> a + b
=> :Int

stdout(safeMath("div", 20, 4).toString())
stdout(safeMath("mod", 17, 5).toString())
stdout(safeMath("add", 3, 7).toString())

// Expected:
// 5
// 2
// 10
