// RC-1n: Diamond dependency — module C (imports D)

>>> ./mod_d.td => @(shared)

_c_helper x = shared(x) => :Str

fromC x = "C:" + _c_helper(x) => :Str

<<< @(fromC)
