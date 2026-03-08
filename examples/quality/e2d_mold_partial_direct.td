trimIt <= Trim[_]()
stdout(trimIt("  hi  "))

splitComma <= Split[_, ","]()
parts <= splitComma("a,b,c")
stdout(parts.length().toString())
