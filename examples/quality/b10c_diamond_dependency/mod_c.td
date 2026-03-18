// B-10c mod_c: imports mod_d (same as mod_b)
>>> ./mod_d => @(shared)

fromC x = "C:" + shared(x) => :Str

<<< @(fromC)
