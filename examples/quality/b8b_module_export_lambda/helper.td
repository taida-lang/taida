// B-8b helper: export BuchiPack containing lambdas
ops <= @(
  add <= _ x y = x + y => :Int,
  mul <= _ x y = x * y => :Int
)

<<< @(ops)
