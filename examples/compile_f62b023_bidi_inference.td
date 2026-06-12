// F62B-023 (a): bidirectional expected-type inference for empty literals.
// An annotated binding, type constructor, or annotated call argument pushes
// its expected type down through pack/list literal trees, so an empty `@[]`
// in a hinted position types as the expected list instead of failing as
// unresolved `@[?]`.

WebRequest = @(path: Str, query: @[@(name: Str, value: Str)])

// Annotated binding: the headline idiom.
req: WebRequest <= @(path <= "/x", query <= @[])
stdout(req.path)
stdout(req.query.length().toString())

// Type constructor.
req2 <= WebRequest(path <= "/y", query <= @[])
stdout(req2.path)
stdout(req2.query.length().toString())

// Annotated call argument.
handle r: WebRequest = r.path => :Str
stdout(handle(@(path <= "/z", query <= @[])))

// Alias inside a pack field expands and hints through.
Pairs = @[@(name: Str, value: Str)]
Box = @(label: Str, items: Pairs)
b: Box <= @(label <= "L", items <= @[])
stdout(b.items.length().toString())

// Nested empties: list-of-lists.
grid: @[@[Int]] <= @[@[], @[1]]
stdout(grid.length().toString())
