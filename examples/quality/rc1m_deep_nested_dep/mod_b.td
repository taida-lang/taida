// RC-1m: Deep nested dependency — module B (imports C)

>>> ./mod_c.td => @(wrap_c)

_internal_b x = "B(" + wrap_c(x) + ")" => :Str

wrap_b x = _internal_b(x) => :Str

<<< @(wrap_b)
