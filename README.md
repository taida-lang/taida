# Taida Lang

Taida Lang is a programming language designed for the AI collaboration era.

It limits itself to 10 operators, has no `null` or `undefined`, and does not allow implicit type conversion. The language is shaped around a simple premise: AI writes it, AI reads it, and humans inspect the structure. That is why Taida is built to make code easy to analyze as graphs.

## What makes it different

- Only 10 operators
- Default values for every type
- Structured data through Buchi Pack `@(...)`
- Parametric typing through molding types `Mold[T]`
- Three backends: Interpreter, JS, and Native
- Structural tooling through `check`, `graph`, `verify`, and `inspect`

## Hello World

```taida
greet name: Str =
  "Hello, " + name + "!"
=> :Str

message <= greet("World")
stdout(message)
```

`stdout` is part of the prelude, so no import is required.

## Small Example

```taida
Pilot = @(
  name: Str
  sync_rate: Int
)

pilot <= Pilot(name <= "Rei", sync_rate <= 78)
stdout(pilot.name)
```

## Usage

The official distribution channel is `install.sh` from `taida.dev`. `crates.io` is not used as a release channel.

Tagged releases are packaged as GitHub Release artifacts for Linux, macOS, and Windows, with a shared `SHA256SUMS` file. `install.sh` should prefer those prebuilt assets and fall back to source build only when a matching artifact does not exist.

To try Taida from source:

```bash
cargo build --release
./target/release/taida examples/01_hello.td
```

Tests:

```bash
cargo test
./tests/run_backend_parity.sh
TAIDA_BIN=./target/release/taida ./tests/e2e_smoke.sh
```

Build outputs:

```bash
./target/release/taida build --target js examples/01_hello.td
./target/release/taida build --target native examples/01_hello.td
```

## Documentation

- [Overview](docs/guide/00_overview.md)
- [Type System](docs/guide/01_types.md)
- [Buchi Pack](docs/guide/04_buchi_pack.md)
- [Molding](docs/guide/05_molding.md)
- [CLI Reference](docs/reference/cli.md)
- [Operators](docs/reference/operators.md)
- [Naming and Versioning](docs/reference/naming_conventions.md)

## Versioning

The canonical public release identifier is the Taida version, not the Rust package semver.

- Canonical public release identifier: `@a.4.alpha`
- `Cargo.toml` version `1.1.1`: semver metadata for Rust tooling

In release notes and public communication, `@a.4.alpha` is the primary version. `1.1.1` is supplementary.
