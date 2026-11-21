# ASCENSION +A

**ASCENSION +A** is a register-based assembly language built for the ASCENSION virtual architecture.

It provides its own instruction set, portable bytecode format, and execution toolchain, supporting interpretation, just-in-time compilation, and native compilation.

+A source files (`.pa`) are assembled into portable ASCENSION Virtual Machine bytecode images (`.avm`). These images can be verified and executed through the AVM runtime, JIT-compiled, or compiled ahead-of-time into standalone x86-64 Linux ELF executables.

## Key Features

- **Virtual Register Architecture** — 32 64-bit general-purpose registers (`R0`–`R31`) with sub-register views (`.b`, `.w`, `.d`, `.q`). Architectural state (`PC`, `FLAGS`, the call stack) is machine state, not addressable registers.
- **Architectural Flags** — Zero (`Z`), Sign (`S`), Carry (`C`), Overflow (`O`), Parity (`P`), and Direction (`D`) flags, with explicit updates through the `!` instruction suffix.
- **Dataflow Arrow Syntax** — Clear `destination <- source` notation, avoiding traditional assembly operand-order ambiguity.
- **Strict Bytecode Verification** — Validation of image magic, format versions, CRC32C section checksums, block jump tables, memory permissions, and instruction encodings.
- **Multi-Tier Execution**
  - **AVM:** Bounded bytecode interpreter with memory protection and fault handling. The reference tier.
  - **JIT:** x86-64 machine-code generation with W^X executable memory management.
  - **Native AOT:** Compilation of bytecode into standalone x86-64 Linux ELF executables.

  The native tiers implement a documented **subset** of the instruction set (36
  of 97 instructions; no memory access, exceptions, or `sys.*`). Requesting a
  native tier for a program outside that subset fails with a named error — it
  never falls back to the interpreter. See
  [`manual/JIT-AOT.md`](manual/JIT-AOT.md).

## Architecture

<pre>
+A Source (.pa)
      |
      v
   Assembler
      |
      v
 AVM Bytecode (.avm)
      |
      v
   Verification
      |
      v
  Execution Engine
   /     |      \
  AVM    JIT    Native AOT
</pre>

## File Extensions

| Extension | Description |
|---|---|
| `.pa` | Official +A source file |
| `.avm` | ASCENSION Virtual Machine bytecode image |

## Quick Start

### Installation

**Method 1: Installer Script (Recommended)**

```bash
./install.sh
```

Installs `+a`, `plus-a`, and the legacy `aa` command into `~/.local/bin/`.

Make sure `~/.local/bin` is included in your `$PATH`.

**Method 2: Cargo**

```bash
cargo install --path rust --bin plus-a --bin aa
```

### Direct Execution

Execute a +A source file directly:

```bash
+a examples/arithmetic.pa
```

### CLI Reference

| Command | Description |
|---|---|
| `+a program.pa` | Build, verify, and execute a source file |
| `+a run program.pa` | Execute +A source |
| `+a run program.avm` | Execute an existing bytecode image |
| `+a build program.pa` | Build bytecode without execution |
| `+a build program.pa -o output.avm` | Build with a custom output path |
| `+a build program.pa --native` | Build a native executable |
| `+a check program.pa` | Parse, assemble, and verify without saving output |
| `+a verify program.avm` | Verify a bytecode image |
| `+a inspect program.avm` | Inspect bytecode structure and metadata |
| `+a run-jit program.avm` | Execute a bytecode image through the JIT |
| `+a compile-native program.avm -o bin` | AOT-compile an image to a native executable |
| `+a run program.pa --jit` | Execute source through the JIT |
| `+a run program.pa --native` | Execute (or `build --native`) as a native binary |
| `+a --version` | Display version information |
| `+a --help` | Display available commands |

## Example Program

```asm
unit arithmetic
export main

main::
    move R0.q <- 100
    move R1.q <- 25
    sum! R0.q <- R0.q, R1.q
    stop
```

**Execution:**

```bash
+a examples/arithmetic.pa
```

**Output:**

```text
stopped: 125
```

## Project Status

**Current release: +A Core V1.1.0.**

The implementation includes the assembler, bytecode format, verification system,
AVM interpreter, JIT compiler, and native AOT backend. The AVM serves as the
reference execution environment; the JIT and native compilation are additional
execution paths over a documented subset of the instruction set.

V1.1.0 is a correctness release: it fixes every defect found in the V1.0.0 audit
and adds permanent regression coverage. See
[`CHANGELOG.md`](CHANGELOG.md) for the complete list of fixes, semantic changes,
and the exact test counts.

`sys.*` services are specified (see [`manual/SYS-ARCHITECTURE.md`](manual/SYS-ARCHITECTURE.md))
but **not implemented**: the capability boundary is enforced and every service is
refused. Implementing them is the scope of "+A Foundation", which has not
started.

## Documentation

Public documentation lives in [`manual/`](manual/README.md):

| Document | Contents |
|---|---|
| [`manual/SPECIFICATION.md`](manual/SPECIFICATION.md) | The +A language specification |
| [`manual/INSTRUCTION-REFERENCE.md`](manual/INSTRUCTION-REFERENCE.md) | All 97 instructions, with per-tier support |
| [`manual/REGISTERS.md`](manual/REGISTERS.md) | Register architecture, views, and flags |
| [`manual/MEMORY-MODEL.md`](manual/MEMORY-MODEL.md) | Memory regions, access rules, atomics |
| [`manual/BYTECODE-FORMAT.md`](manual/BYTECODE-FORMAT.md) | The `.avm` container, byte for byte |
| [`manual/VERIFICATION.md`](manual/VERIFICATION.md) | Every verification rule |
| [`manual/AVM.md`](manual/AVM.md) | The interpreter |
| [`manual/JIT-AOT.md`](manual/JIT-AOT.md) | Native tiers: subset, ABI, limits |
| [`manual/CLI.md`](manual/CLI.md) | Command-line reference |
| [`manual/BUILD-AND-INSTALL.md`](manual/BUILD-AND-INSTALL.md) | Build, install, test, troubleshoot |
| [`manual/SYS-ARCHITECTURE.md`](manual/SYS-ARCHITECTURE.md) | `sys.*` capability design (design only) |
| [`manual/KNOWN-LIMITATIONS.md`](manual/KNOWN-LIMITATIONS.md) | What V1.1.0 cannot do |

These documents are checked against the implementation where the claim is
machine-checkable; see [`manual/README.md`](manual/README.md) for the test names.

## Testing & Quality Assurance

Run the complete test suite:

```bash
cargo test --manifest-path rust/Cargo.toml --locked --offline --lib --tests
```

`--lib --tests` avoids doc-tests, which need `rustdoc`; some minimal toolchains
omit it and the failure looks like a test failure.

Run Clippy:

```bash
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
```

Check formatting:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
```

## License

ASCENSION is distributed under the custom source-available license in [`LICENSE`](LICENSE).

You may use the ASCENSION toolchain and freely create, modify, compile, execute, and distribute your own +A programs.

Modification, forking, and redistribution of modified versions of the ASCENSION implementation are not permitted under the license.

---

**ASCENSION**

*Building the foundations of a language family.*