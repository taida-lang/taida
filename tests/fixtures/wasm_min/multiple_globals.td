// W-2 regression test: multiple global variables should not collide.
// Each global gets its own C static variable.
x <= 10
y <= 20
z <= 30
debug(x + y + z)
