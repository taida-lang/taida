// W-4: Nested BuchiPack test
inner <= @(x <= 10, y <= 20)
outer <= @(label <= "test", data <= inner)
stdout(outer.label)
debug(outer.data.x)
debug(outer.data.y)
