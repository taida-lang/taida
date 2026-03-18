// RC-1m: Deep nested dependency — module A (imports B)

>>> ./mod_b.td => @(wrap_b)

wrap_a x = "A(" + wrap_b(x) + ")" => :Str

<<< @(wrap_a)
