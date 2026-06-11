// compile_b11_features.td — B11 new features demo (@b.11.rc3)
// Demonstrates: replace/split methods, If mold, TypeIs/TypeExtends

// ── Str replace / replaceAll / split ────────────────────

// replace: first match only
msg <= "hello world hello"
stdout(msg.replace("hello", "hi"))

// replaceAll: all matches
stdout(msg.replaceAll("hello", "hi"))

// split: by separator
parts <= "a,b,c".split(",")
stdout(parts.length())

// split: empty separator splits into characters
chars <= "abc".split("")
stdout(chars.length())

// ── If mold ─────────────────────────────────────────────

// Basic true/false
stdout(If[true, "yes", "no"]())
stdout(If[false, "yes", "no"]())

// Pipeline clamp: one `_` per stage (F62B-025 rule 1) — re-using the
// piped value in several positions goes through a `=> v` bind-and-forward
150 => v => If[v > 100, 100, v]() => clamped
stdout(clamped)

50 => w => If[w > 100, 100, w]() => not_clamped
stdout(not_clamped)

// Nested If
stdout(If[true, If[false, 1, 2](), 3]())

// ── TypeIs ──────────────────────────────────────────────

stdout(TypeIs[42, :Int]())
stdout(TypeIs["hello", :Str]())
stdout(TypeIs[true, :Bool]())
stdout(TypeIs[42, :Str]())

// Enum variant check
Enum => Color = :Red :Green :Blue
c <= Color:Green()
stdout(TypeIs[c, Color:Green]())
stdout(TypeIs[c, Color:Red]())

// ── TypeExtends ─────────────────────────────────────────

stdout(TypeExtends[:Int, :Num]())
stdout(TypeExtends[:Float, :Num]())
stdout(TypeExtends[:Str, :Int]())

// Named type inheritance
Animal = @(name: Str)
Animal => Dog = @(breed: Str)
stdout(TypeExtends[:Dog, :Animal]())
stdout(TypeExtends[:Animal, :Dog]())
