// RC-1n: Diamond dependency — module B (imports D)

>>> ./mod_d.td => @(shared)

_b_helper x = shared(x) => :Str

fromB x = "B:" + _b_helper(x) => :Str

<<< @(fromB)
