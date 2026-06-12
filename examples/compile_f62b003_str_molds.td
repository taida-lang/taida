// F62B-003: search / replace / pad string molds (prelude.md §7.4) must
// return their documented types on every backend — these previously fell
// through to the generic mold fallback (interpreter: unresolved
// `@(__value, __type)` pack; native/wasm: "unsupported mold type").
// Each mold form must agree with its method form.

s <= "hello world"

stdout(IndexOf[s, "world"]().toString())
stdout(s.indexOf("world").toString())

stdout(LastIndexOf[s, "l"]().toString())
stdout(s.lastIndexOf("l").toString())

stdout(Contains[s, "wor"]().toString())
stdout(s.contains("wor").toString())

stdout(ReplaceAll[s, "l", "L"]())
stdout(s.replaceAll("l", "L"))

stdout(PadLeft["7", 3, "0"]())
stdout(PadRight["7", 3, "0"]())
stdout(Pad["7", 3](side <= "start", char <= "0"))

// not-found sentinel parity
stdout(IndexOf[s, "zzz"]().toString())
stdout(Contains[s, "zzz"]().toString())
