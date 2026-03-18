// D-1g: docs/guide/06_lists.md code examples

// List literals
numbers <= @[1, 2, 3, 4, 5]
names <= @["Asuka", "Rei", "Shinji"]
empty: @[Int] <= @[]
nested <= @[@[1, 2], @[3, 4]]

// State check methods
stdout(@[1, 2, 3].length().toString())
stdout(empty.length().toString())
stdout(empty.isEmpty().toString())
stdout(@[1, 2].isEmpty().toString())
stdout(@[1, 2, 3].contains(2).toString())
stdout(@[1, 2, 3].contains(99).toString())

// indexOf
stdout(@[10, 20, 30, 20].indexOf(20).toString())
stdout(@[10, 20, 30].indexOf(99).toString())

// Safe access (Lax)
@[10, 20, 30].get(1) ]=> got_val
stdout(got_val.toString())
@[10, 20, 30].get(100) ]=> got_oob
stdout(got_oob.toString())
@[1, 2, 3].first() ]=> first_val
stdout(first_val.toString())
empty.first() ]=> empty_first
stdout(empty_first.toString())
@[1, 2, 3].last() ]=> last_val
stdout(last_val.toString())
@[1, 3, 2].max() ]=> max_val
stdout(max_val.toString())
@[3, 1, 2].min() ]=> min_val
stdout(min_val.toString())

// any/all/none
stdout(@[1, 2, 3, 4].any(_ x = x > 3).toString())
stdout(@[1, 2, 3, 4].all(_ x = x > 0).toString())
stdout(@[1, 2, 3, 4].none(_ x = x > 10).toString())

// Operation molds
Map[@[1, 2, 3], _ x = x * 2]() ]=> mapped
stdout(mapped)
Filter[@[1, 2, 3, 4, 5], _ x = x > 3]() ]=> filtered
stdout(filtered)
Fold[@[1, 2, 3, 4, 5], 0, _ acc x = acc + x]() ]=> total
stdout(total.toString())
Sort[@[3, 1, 2]]() ]=> sorted
stdout(sorted)
Reverse[@[1, 2, 3]]() ]=> reversed
stdout(reversed)
Concat[@[1, 2], @[3, 4]]() ]=> concatted
stdout(concatted)
Join[@["a", "b", "c"], ","]() ]=> joined
stdout(joined)
Append[@[1, 2], 3]() ]=> appended
stdout(appended)
Prepend[@[1, 2], 0]() ]=> prepended
stdout(prepended)
Sum[@[1, 2, 3, 4, 5]]() ]=> sum_val
stdout(sum_val.toString())
Unique[@[1, 2, 2, 3, 3]]() ]=> unique_val
stdout(unique_val)
Take[@[1, 2, 3, 4, 5], 3]() ]=> taken
stdout(taken)
Drop[@[1, 2, 3, 4, 5], 3]() ]=> dropped
stdout(dropped)
Find[@[1, 2, 3, 4, 5], _ x = x > 3]() ]=> found
stdout(found.toString())
FindIndex[@[10, 20, 30], _ x = x > 15]() ]=> found_idx
stdout(found_idx.toString())
Count[@[1, 2, 3, 4, 5], _ x = x > 3]() ]=> counted
stdout(counted.toString())

// Pipeline
numbers => Filter[_, _ x = x > 2]() => Map[_, _ x = x * 10]() => pipe_result
stdout(pipe_result)

// Flatten
Flatten[@[@[1, 2], @[3, 4]]]() ]=> flat
stdout(flat)
