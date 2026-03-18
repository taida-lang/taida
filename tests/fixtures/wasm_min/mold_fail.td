// W-5g: Type mold failures should return empty Lax
stdout(Float["bad"]().toString())
stdout(Bool["bad"]().toString())

// Boundary cases: partially-valid tokens must also fail
stdout(Float["1e"]().toString())
stdout(Float["1e+"]().toString())
stdout(Float["."]().toString())

// Valid cases
stdout(Float["3.14"]().toString())
stdout(Float["1e2"]().toString())
stdout(Bool["true"]().toString())
