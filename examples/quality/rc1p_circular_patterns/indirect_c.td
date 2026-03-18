// RC-1p: Indirect circular — C -> A (completes the cycle!)
>>> ./indirect_a.td => @(fromA)
fromC x = "C:" + fromA(x) => :Str
<<< @(fromC)
