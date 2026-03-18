// Quality Test: B-9a — BuchiPack 自己参照
// 不変セマンティクスにより自己参照は不可能
// 代入時にスナップショットコピーされるため循環しない

inner <= @(val <= 10)
outer <= @(child <= inner, name <= "outer")
stdout(outer.child.val.toString())
stdout(outer.name)
// Expected:
// 10
// outer
