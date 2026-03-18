// Quality Test: B-7j — TypeDef + 継承 + メソッドオーバーライド
// InheritanceDef のメソッドから親フィールド + 子フィールド参照

Vehicle = @(
  name: Str
  speed: Int
  describe =
    name + ":" + speed.toString()
  => :Str
)

Vehicle => ElectricCar = @(
  battery: Int
  describe =
    name + ":" + speed.toString() + ":bat=" + battery.toString()
  => :Str
  status =
    name + " battery:" + battery.toString()
  => :Str
)

v <= Vehicle(name <= "Truck", speed <= 80)
e <= ElectricCar(name <= "Tesla", speed <= 200, battery <= 95)
stdout(v.describe())
stdout(e.describe())
stdout(e.status())

// Expected: Truck:80
// Expected: Tesla:200:bat=95
// Expected: Tesla battery:95
