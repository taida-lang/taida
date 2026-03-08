// Quality Test: B-6f — BuchiPack 内 TypeDef インスタンス化
// フィールド値として TypeDef インスタンスを格納

Dog = @(
  name: Str
  age: Int
  bark =
    "Woof:" + name
  => :Str
)

bag <= @(
  pet <= Dog(name <= "Rex", age <= 3),
  label <= "myPet"
)

stdout(bag.label)
stdout(bag.pet.name)
stdout(bag.pet.age.toString())
stdout(bag.pet.bark())
// Expected: myPet
// Expected: Rex
// Expected: 3
// Expected: Woof:Rex
