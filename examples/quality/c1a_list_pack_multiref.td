// Quality Test: C-1a — List 内 BuchiPack を関数から返却 → 2 回以上フィールドアクセス（F-48 一般化）
makePeople =
  @[
    @(name <= "Taro", age <= 25),
    @(name <= "Hana", age <= 30),
    @(name <= "Ken", age <= 18)
  ]
=> :@[@(name: Str, age: Int)]

people <= makePeople()
stdout(people.get(0).getOrDefault(@(name <= "", age <= 0)).name)
stdout(people.get(0).getOrDefault(@(name <= "", age <= 0)).age.toString())
stdout(people.get(1).getOrDefault(@(name <= "", age <= 0)).name)
stdout(people.get(2).getOrDefault(@(name <= "", age <= 0)).name)
stdout(people.get(2).getOrDefault(@(name <= "", age <= 0)).age.toString())
// Expected:
// Taro
// 25
// Hana
// Ken
// 18
