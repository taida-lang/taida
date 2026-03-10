// W-5g F-2: Type mold failures should return empty Lax
// Float["bad"]() should be Lax(default: 0)
stdout(Float["bad"]().toString())

// Bool["bad"]() should be Lax(default: 0)
stdout(Bool["bad"]().toString())

// Float["3.14"]() should succeed
stdout(Float["3.14"]().toString())

// Bool["true"]() should succeed
stdout(Bool["true"]().toString())
