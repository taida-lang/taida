// RC-1p: Indirect circular — B -> C
>>> ./indirect_c.td => @(fromC)
fromB x = "B:" + fromC(x) => :Str
<<< @(fromB)
