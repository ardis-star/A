# `+a` Command-Line Reference

**Edition:** ASCENSION +A Core V1.1.0

The toolchain installs three names for the same binary: `plus-a`, `aa`, and the
shell-friendly `+a` symlink.

```
+a <source.pa>                      Build, verify and execute directly
+a run <source.pa|image.avm>        Explicitly execute (+A source or .avm bytecode)
+a build <source.pa> [OPTIONS]      Build without executing
+a check <source.pa>                Parse, assemble and verify without saving
+a verify <image.avm>               Verify an existing .avm bytecode image
+a inspect <image.avm>              Inspect bytecode structure and sections
+a run-jit <image.avm>              Execute bytecode image using the JIT compiler
+a compile-native <image> -o <bin>  AOT-compile bytecode image to native executable
+a --version                        Show version information
+a --help                           Show this help message
```

## Options

| Option | Applies to | Meaning |
|---|---|---|
| `-o`, `--output <path>` | `build`, `compile-native`, `<source>` | Output path. With `build` and no `--native`, writes a bytecode image and prints `built: <path>`. |
| `--build-dir <dir>` | `run`, `build`, `<source>` | Where generated files go. Default `./build`; bytecode lands in `<dir>/bytecode/`, native binaries in `<dir>/native/`. |
| `--native`, `-n` | `run`, `build` | Compile and run (or just compile) as a native x86-64 executable. |
| `--jit` | `run` | Execute through the JIT instead of the interpreter. |

`--jit` and `--native` are mutually exclusive; combining them is an error
(`'--jit' and '--native' select different execution modes and cannot be
combined`).

## Commands in detail

### `+a <source.pa>`

Assembles, verifies, writes `<build-dir>/bytecode/<stem>.avm`, then runs it in
the interpreter and prints the result. With `-o` it writes bytecode to that path
and **does not execute**, printing `built: <path>`.

### `+a run <file> [--jit|--native] [--build-dir <dir>]`

Accepts either a `.pa` source or a `.avm` image. Without a tier flag it
interprets. `--native` works for **both** inputs: for a source it assembles
first, for an image it uses the image directly. Requesting a native tier for an
image outside the [native subset](JIT-AOT.md) fails with status 2 and names the
offending instructions.

Output:

| Tier | Printed |
|---|---|
| interpreter | `stopped: <status>` or `returned` |
| JIT | `jit stopped: <64-bit status>` |
| AOT | `stopped: <status & 0xFF>` (a process exit status is 8 bits) |

### `+a build <source.pa> [-o <path>] [--native]`

Produces an artifact without running it. Bytecode by default, a native
executable with `--native`. Prints `built: <path>`.

### `+a check <source.pa>`

Lexes, parses, assembles and verifies, writing nothing, and prints
`checked: <path>`. The fastest way to validate a program in CI.

### `+a verify <image.avm>`

Runs the full verifier. Prints `verified: N function(s), entry Some(0)` on
success. See [VERIFICATION.md](VERIFICATION.md).

### `+a inspect <image.avm>`

Structural decode only, then a section and function summary. It deliberately
does not require execution-verifiability, so it can describe an image that
`verify` would reject.

### `+a run-jit <image.avm>`

JIT-compiles and runs an image, printing `jit stopped: <status>`.

### `+a compile-native <image> -o <bin>`

AOT-compiles an image to a standalone executable. Verifies the image and checks
the native subset before invoking `cc`, so an unsupported image never produces a
partial binary. Prints `built: <path>`.

## Exit statuses

| Status | Meaning |
|---|---|
| `0` | Success. |
| `2` | Any diagnosed failure: assembly error, verification failure, unsupported native instruction, missing file, bad option. The message goes to stderr. |
| `70` | Emitted by a **generated native executable** when it faults at run time. The CLI maps this back to `error: runtime fault: …` and exits `2`. |
| other | A generated native executable returned this value as its `stop` status (truncated to 8 bits). |

`+a --version` prints `+a 1.1.0`.

## File extensions

| Extension | Meaning |
|---|---|
| `.pa` | +A source (official). |
| `.a` | Legacy source spelling, still accepted. |
| `.ma` | Rejected: that suffix belongs to the unrelated `-A` project. |
| `.avm` | Bytecode image. |

## Diagnostics format

Source diagnostics are `path:line:column: message`:

```
program.pa:3:12: does not fit 8 bits
```

Verification and I/O diagnostics are `path: message`:

```
program.avm: declared file size does not match image
```

## Worked example

```console
$ +a build examples/hello_world.pa --build-dir out
built: out/bytecode/hello_world.avm
$ +a verify out/bytecode/hello_world.avm
verified: 1 function(s), entry Some(0)
$ +a run out/bytecode/hello_world.avm
stopped: 0
$ +a run out/bytecode/hello_world.avm --jit
jit stopped: 0
$ +a build examples/hello_world.pa --native -o out/hello
built: out/hello
$ out/hello; echo $?
0
```
