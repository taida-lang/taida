// Quality Test: B-9c — closure 自己参照（再帰呼び出し）
// 関数が自身を名前で再帰呼び出しするパターン

countdown n =
  | n < 1 |> "done"
  | _ |> countdown(n - 1)
=> :Str

stdout(countdown(5))

// カウンターパターン: closure が自身を参照せず外の関数名で再帰
sumRec n acc =
  | n == 0 |> acc
  | _ |> sumRec(n - 1, acc + n)
=> :Int

stdout(sumRec(10, 0).toString())
// Expected:
// done
// 55
