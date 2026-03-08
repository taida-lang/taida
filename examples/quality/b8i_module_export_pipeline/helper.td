// B-8i helper: export function with pipeline
processNums nums =
  Filter[nums, _ x = Mod[x, 2]().unmold() == 0]() ]=> evens
  Map[evens, _ x = x * 10]() ]=> scaled
  Sum[scaled]() ]=> total
  total
=> :Int

<<< @(processNums)
