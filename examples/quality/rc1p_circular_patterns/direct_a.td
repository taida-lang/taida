// RC-1p: Direct circular — A imports B, B imports A
>>> ./direct_b.td => @(fromB)
fromA x = "A:" + fromB(x) => :Str
<<< @(fromA)
