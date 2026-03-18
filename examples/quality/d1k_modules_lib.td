// D-1k helper: Module library for import test
helper x: Int =
  x * 10
=> :Int

FORMAT_PREFIX <= "Result: "

<<< @(helper, FORMAT_PREFIX)
