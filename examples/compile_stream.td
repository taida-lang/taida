// compile_stream.td -- Stream[T] mold type test
// Tests: Stream, StreamFrom, Filter, Map, Take, TakeWhile, pipeline, unmold

// helpers
isEven x =
  Mod[x, 2]() ]=> r
  r == 0
=> :Bool
double x = x * 2 => :Int
lessThan5 x = x < 5 => :Bool
timesTen x = x * 10 => :Int

// -- Stream[value]() -- single value stream
s1 <= Stream[42]()
s1 ]=> r1
stdout(Join[r1, " "]())

// -- StreamFrom[list]() -- stream from list
nums <= @[1, 2, 3, 4, 5]
s2 <= StreamFrom[nums]()
s2 ]=> r2
stdout(Join[r2, " "]())

// -- Map[stream, fn]() -- lazy map
s3 <= StreamFrom[@[10, 20, 30]]()
mapped <= Map[s3, double]()
mapped ]=> r3
stdout(Join[r3, " "]())

// -- Filter[stream, fn]() -- lazy filter
s4 <= StreamFrom[@[1, 2, 3, 4, 5, 6]]()
filtered <= Filter[s4, isEven]()
filtered ]=> r4
stdout(Join[r4, " "]())

// -- Take[stream, n]() -- lazy take
s5 <= StreamFrom[@[10, 20, 30, 40, 50]]()
taken <= Take[s5, 3]()
taken ]=> r5
stdout(Join[r5, " "]())

// -- TakeWhile[stream, pred]() -- lazy take while
s6 <= StreamFrom[@[1, 2, 3, 10, 20]]()
tw <= TakeWhile[s6, lessThan5]()
tw ]=> r6
stdout(Join[r6, " "]())

// -- Chained transforms: Filter => Map => Take
s7 <= StreamFrom[@[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]]()
step1 <= Filter[s7, isEven]()
step2 <= Map[step1, timesTen]()
step3 <= Take[step2, 3]()
step3 ]=> results
stdout(Join[results, " "]())

// -- Stream methods
s8 <= StreamFrom[@[1, 2, 3]]()
stdout(s8.toString())
len8 <= s8.length()
stdout(len8.toString())
empty8 <= s8.isEmpty()
stdout(empty8.toString())

// -- Empty stream
s9 <= StreamFrom[@[]]()
empty9 <= s9.isEmpty()
stdout(empty9.toString())
