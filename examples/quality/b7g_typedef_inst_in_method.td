// Quality Test: B-7g — TypeDef メソッド内 TypeDef インスタンス化
// メソッド内で自型/他型のインスタンスを生成

Tag = @(label: Str)

Item = @(
  name: Str
  makeTag =
    Tag(label <= "tag:" + name)
  => :Tag
)

it <= Item(name <= "alpha")
t <= it.makeTag()
stdout(t.label)

// Expected: tag:alpha
