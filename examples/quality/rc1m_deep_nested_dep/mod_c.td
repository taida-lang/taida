// RC-1m: Deep nested dependency — module C (imports D)

>>> ./mod_d.td => @(wrap_d)

_internal_c x = "C(" + wrap_d(x) + ")" => :Str

wrap_c x = _internal_c(x) => :Str

<<< @(wrap_c)
