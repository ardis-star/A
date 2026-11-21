# ASCENSION +A

**ASCENSION +A** is a register-based assembly language built for the ASCENSION virtual architecture.

It provides its own instruction set, portable bytecode format, and execution toolchain, supporting interpretation, just-in-time compilation, and native compilation.

+A source files (`.pa`) are assembled into portable ASCENSION Virtual Machine bytecode images (`.avm`). These images can be verified and executed through the AVM runtime, JIT-compiled, or compiled ahead-of-time into standalone x86-64 Linux ELF executables.

## Key Features

- **Virtual Register Architecture** — 32 64-bit general-purpose registers (`R0`–`R31`) with subregister views (`.b`, `.w`, `.d`, `.q`), alongside dedicated `SP`, `PC`, `FP`, and `FLAGS` registers.
- **Architectural Flags** — Zero (`Z`), Sign (`S`), Carry (`C`), Overflow (`O`), Parity (`P`), and Direction (`D`) flags, with explicit updates through the `!` instruction suffix.
- **Dataflow Arrow Syntax** — Clear `destination <- source` notation, avoiding traditional assembly operand-order ambiguity.
- **Strict Bytecode Verification** — Validation of image magic, format versions, CRC32C section checksums, block jump tables, memory permissions, and instruction encodings.
- **Multi-Tier Execution**
  - **AVM:** Bounded bytecode interpreter with memory protection and fault handling.
  - **JIT:** x86-64 machine-code generation with W^X executable memory management.
  - **Native AOT:** Compilation of bytecode into standalone x86-64 Linux ELF executables.

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

ASCENSION +A is under active development.

The current implementation includes the assembler, bytecode format, verification system, AVM interpreter, JIT compiler, and native AOT backend.

The AVM serves as the reference execution environment, while JIT and native compilation provide additional execution paths.

## Documentation

The public repository contains the source code, examples, README, and test suite.

Detailed architectural specifications and internal design documentation are maintained separately for future publication.

## Testing & Quality Assurance

Run the complete test suite:

```bash
cargo test --manifest-path rust/Cargo.toml
```

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