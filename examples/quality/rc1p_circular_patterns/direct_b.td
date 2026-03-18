// RC-1p: Direct circular — B imports A (circular!)
>>> ./direct_a.td => @(fromA)
fromB x = "B:" + x => :Str
<<< @(fromB)
