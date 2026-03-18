// RC-1p: Indirect circular — A -> B -> C -> A
>>> ./indirect_b.td => @(fromB)
fromA x = "A:" + fromB(x) => :Str
<<< @(fromA)
