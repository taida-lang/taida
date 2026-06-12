// F62B-025: pipeline two-rule semantics — backend parity fixture.
// Rule 1: a stage with `_` (one at most) injects the piped value there.
// Rule 2: a `_`-free stage is evaluated as written; a function result
//         receives the piped value (compositionality: inline partial
//         application ≡ bound closure).

add x: Int  y: Int =
  x + y
=> :Int

addThree x: Int  y: Int  z: Int =
  x + y + z
=> :Int

double n: Int =
  n * 2
=> :Int

// Rule 2: inline empty-slot partial application evaluates to a closure,
// the piped value is applied to it.
5 => add(, 3) => r1
stdout(r1.toString())

// Rule 1: `_` marks the injection position.
5 => add(_, 3) => r2
stdout(r2.toString())

// Rule 2: bare function name.
5 => double => r3
stdout(r3.toString())

// Rule 1: single-argument placeholder form.
5 => double(_) => r4
stdout(r4.toString())

// Compositionality: the bound-closure form gives the same result as r1.
f <= add(, 3)
5 => f => r5
stdout(r5.toString())

// Rule 1 + empty slot: positional injection — `_` takes the piped value,
// the hole stays open, the stage yields a closure.
5 => addThree(, _, 3) => g
g(100) => r6
stdout(r6.toString())

// Rule 1 in method arguments.
nums <= @[30, 10, 20]
1 => nums.get(_) => v
stdout(v.getOrDefault(-1).toString())

// Rule 1 in a bare operator stage (`_` carries the piped type).
5 => _ + 1 => r7
stdout(r7.toString())

// Rule 1 in mold stages.
"  hi  " => Trim[_]() => t
stdout(t)

// Chained stages mixing both rules.
2 => double => add(, 1) => double(_) => r8
stdout(r8.toString())
