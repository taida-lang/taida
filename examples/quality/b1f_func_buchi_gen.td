// B-1f: 関数内 BuchiPack 生成 — ネスト BuchiPack を生成して返却
makeProfile name age =
  @(
    name <= name,
    age <= age,
    meta <= @(active <= true, tag <= "user")
  )
=> :@(name: Str, age: Int, meta: @(active: Bool, tag: Str))

profile <= makeProfile("Asuka", 14)
stdout(profile.name)
stdout(profile.age.toString())
stdout(profile.meta.tag)
// Expected: Asuka
// Expected: 14
// Expected: user
