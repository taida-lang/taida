// C24-B: enumerate on an empty list returns an empty list.
xs: @[Str] <= @[]
stdout(Str[Enumerate[xs]()]())
