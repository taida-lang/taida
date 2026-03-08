// Quality Test: B-9j — 関数引数の BuchiPack/List が呼び出し後も生存
// release タイミングが正しいことを確認

readName obj = obj.name => :Str
readFirst items = items.get(0).getOrDefault(0) => :Int

person <= @(name <= "Alice", age <= 30)
nums <= @[100, 200, 300]

// 関数に渡した後も生存しているか
stdout(readName(person))
stdout(person.name)
stdout(person.age.toString())

stdout(readFirst(nums).toString())
stdout(nums.length().toString())
stdout(nums)
// Expected:
// Alice
// Alice
// 30
// 100
// 3
// @[100, 200, 300]
