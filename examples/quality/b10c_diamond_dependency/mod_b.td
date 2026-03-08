// B-10c mod_b: imports mod_d
>>> ./mod_d => @(shared)

fromB x = "B:" + shared(x) => :Str

<<< @(fromB)
