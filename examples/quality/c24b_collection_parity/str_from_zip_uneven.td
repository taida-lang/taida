// C24-B: zip truncates to the shorter input.
a <= @[1, 2, 3, 4, 5]
b <= @["x", "y"]
stdout(Str[Zip[a, b]()]())
