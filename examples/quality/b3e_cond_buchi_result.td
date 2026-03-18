// B-3e: BuchiPack as pattern match result

makePoint flag =
  | flag |> @(x <= 10, y <= 20)
  | _ |> @(x <= 0, y <= 0)
=> :@(x: Int, y: Int)

p1 <= makePoint(true)
p2 <= makePoint(false)
stdout(p1.x.toString() + "," + p1.y.toString())
stdout(p2.x.toString() + "," + p2.y.toString())

// Expected:
// 10,20
// 0,0
