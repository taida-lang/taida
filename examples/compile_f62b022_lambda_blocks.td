// F62B-022: block-bodied lambdas. With a newline after `=`, a lambda body
// is an indented statement block (let-bindings + a final result expression
// or tail binding — the same pure-expression discipline as `| |>` arm
// bodies). Block-bodied lambdas require type annotations on every
// parameter. Single-expression lambdas are unchanged.

toLabel <= _ n: Int =
  doubled <= n * 2
  suffix <= "x"
  `${doubled}${suffix}`

stdout(toLabel(21))

nums <= @[1, 2, 3]
mapped <= Map[nums, toLabel]()
stdout(mapped.toString())

// unmold bindings inside the block
halve <= _ n: Int =
  Div[n, 2]() >=> half
  half

stdout(halve(10).toString())

// tail-binding final statement yields the bound value
pick <= _ n: Int =
  chosen <= n + 100

stdout(pick(1).toString())

// single-expression lambdas still infer from context
plain <= Map[nums, _ x: Int = x * 10]()
stdout(plain.toString())
