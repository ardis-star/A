# Building, Installing, and Testing +A

**Edition:** ASCENSION +A Core V2.0.0

## 1. Requirements

| Requirement | Version | Needed for |
|---|---|---|
| Rust | **1.87 or newer** (edition 2024 needs 1.85; the code uses `u64::is_multiple_of`, stable in 1.87). V2 changes were also tested with the pinned 1.97.0 | building the toolchain |
| `cc` (any C toolchain) | — | **only** the JIT and AOT backends; the interpreter does not need it |
| Linux x86-64 | — | the JIT (uses `dlopen`) and the AOT backend (emits SysV x86-64 assembly) |

There are **no third-party crate dependencies**. `rust/Cargo.toml` declares an
empty `[dependencies]` table and `rust/Cargo.lock` pins only the crate itself,
so a build never contacts a registry.

`rust/rust-toolchain.toml` pins channel `1.97.0` with `rustfmt` and `clippy`.
With rustup installed, entering `rust/` selects that toolchain automatically.
Without rustup, any 1.87+ compiler works.

## 2. Building

```bash
cd rust
cargo build --locked --offline           # debug
cargo build --locked --offline --release # release
```

`--locked` guarantees the checked-in `Cargo.lock` is used unchanged; `--offline`
guarantees nothing is fetched. Both are unnecessary on a normal machine but are
what the release was validated with.

Binaries: `rust/target/{debug,release}/plus-a` and `.../aa` (same program).

### 2.1 Reproducibility

* **Bytecode is reproducible.** Assembling the same source twice yields
  byte-identical images, pinned by golden tests and by
  `repeated_assembly_is_byte_for_byte_identical`.
* **The compiler version is pinned** (`rust-toolchain.toml`), so diagnostics and
  generated code do not drift with the toolchain.
* **Native binaries are not reproducible**, because linking is delegated to the
  host `cc`. The generated assembly *text* is deterministic; the ELF is not.

## 3. Installing

```bash
./install.sh
```

Builds in release mode and installs `plus-a`, `aa`, and a `+a` symlink into
`${INSTALL_DIR:-$HOME/.local/bin}`. Ensure that directory is on `PATH`.

Alternative, without the script:

```bash
cargo install --path rust --bin plus-a --bin aa
```

The repository also ships a `+a` wrapper script that runs the release binary if
present, otherwise the debug binary, and builds one if neither exists. Use it
directly from a checkout:

```bash
./+a examples/hello_world.pa
```

## 4. Testing

```bash
cd rust
cargo test --locked --offline
```

> Full release validation includes doc-tests and requires `rustdoc`. The
> pinned toolchain validation is recorded in [V2-ACCEPTANCE.md](V2-ACCEPTANCE.md).

The suite is organised as:

| Target | What it covers |
|---|---|
| `tests/assembler.rs` | lexer, parser, encoder, golden images, CLI end-to-end |
| `tests/avm.rs` | interpreter semantics, faults, memory, exceptions |
| `tests/verifier.rs` | verification rules and malformed-image cases |
| `tests/cli.rs` | command surface, exit statuses, diagnostics |
| `tests/compatibility.rs` | representative V1.1.0 bytecode fixtures |
| `tests/regressions.rs` | every bug fixed in V1.1.0/V2.0.0, tier-equivalence, fuzzing, docs/code agreement |
| `tests/golden/*.a` | exact-byte fixtures |

V2.0.0 reports **182 passing tests, 1 ignored** across these targets (see
[CHANGELOG.md](../CHANGELOG.md) for the exact per-target counts). The ignored
test requires a Python reference source tree absent from this checkout.

### 4.1 Formatting and lints

```bash
cargo fmt -- --check
cargo clippy --locked --offline --all-targets --all-features -- -D warnings
```

Note that `cargo fmt` takes no `--offline`; it runs the `rustfmt` binary.

### 4.2 Tests that need `cc`

The JIT and AOT tests invoke `cc` through the library. If `cc` is missing, those
tests fail with a linker diagnostic rather than being skipped — the suite
assumes a usable C toolchain on Linux x86-64. The JIT tests are compiled out
entirely on other platforms.

## 5. Cross-platform notes

| Platform | Interpreter / assembler / verifier | JIT | AOT |
|---|---|---|---|
| Linux x86-64 | yes | yes (needs `cc`) | yes (needs `cc`) |
| Linux aarch64 | not validated | no — `run_jit` returns an error explaining the limitation | no |
| macOS | not validated | no | not validated |
| Windows | not validated; crate currently uses Unix-specific APIs | no | not validated |

The backend-dependent functions return a diagnosable error rather than being
silently reinterpreted, so a program that asks for JIT on an unsupported
platform fails with
`the current JIT requires Linux x86-64 and a local cc toolchain`.

## 6. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: could not compile ... edition 2024` | Rust older than 1.85 | install 1.97.0 or newer |
| `error: current MSRV ... stable since 1.87.0` | Rust older than 1.87 | install 1.97.0 or newer |
| `rustdoc` not found | incomplete Rust installation | install the complete pinned toolchain |
| `jit error: the current JIT requires…` | not Linux x86-64 | use the interpreter |
| `failed to start cc` | no C toolchain | install `cc`, or avoid `--jit`/`--native` |
| `error: this image cannot run natively` | image uses interpreter-only instructions | run without `--native`; see [JIT-AOT.md](JIT-AOT.md) |
| `MEMORY_ACCESS at 0x…` from a memory program | the CLI maps no regions | expected; embed the AVM and map regions |
