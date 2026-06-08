// F56 secret carrier (Level 0): producer + Redact mask.
//
// `MoltenizeSecret[v]()` / `Moltenize[v]()` seal a value in an opaque carrier
// (`Secret[T]` / `Moltenized[T]`). The sealed value can never be displayed,
// serialized, compared, or unmolded directly — only an explicit `Redact[]`
// (fixed "***" mask) or a secret-aware consumer may observe it. This fixture
// pins identical output across interpreter / JS / native / wasm.
secret <= MoltenizeSecret["api-key"]()
stdout(Redact[secret]())

opaque <= Moltenize[42]()
stdout(Redact[opaque]())
