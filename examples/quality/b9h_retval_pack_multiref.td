// Quality Test: B-9h — 関数から返した BuchiPack を複数回参照
// retain/release が正しく動くことを確認

makePerson name age = @(name <= name, age <= age) => :@(name: Str, age: Int)

p <= makePerson("Taro", 25)
stdout(p.name)
stdout(p.age.toString())
stdout(p.name)
stdout(p.age.toString())

// 別変数に代入して両方から参照
q <= p
stdout(q.name)
stdout(p.name)
// Expected:
// Taro
// 25
// Taro
// 25
// Taro
// Taro
