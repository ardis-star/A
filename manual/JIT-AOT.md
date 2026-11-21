# JIT and AOT Backends: Capabilities and Limitations

**Edition:** ASCENSION +A Core V1.1.0

The toolchain has three execution tiers. This document states precisely what the
two native tiers do, what they cannot do, and how a refusal is reported.

| Tier | Command | Mechanism |
|---|---|---|
| Interpreter | `+a run <file>` (default), `+a <source.pa>` | `ascension_asm::avm::Avm` |
| JIT | `+a run <file> --jit`, `+a run-jit <image>` | emits x86-64 assembly, assembles and links a shared object with `cc`, `dlopen`s it, calls the entry point |
| AOT | `+a run <file> --native`, `+a build --native`, `+a compile-native` | emits x86-64 assembly, assembles and links a standalone ELF executable with `cc` |

## 1. The native subset

The native backends lower **36 of the 97** defined instructions. The subset is
declared once, in `NATIVE_OPCODE_RANGES` (`rust/src/native.rs`), and
`native_opcode_table_matches_the_emitter` fails if the emitter and the table
ever disagree. The authoritative per-instruction table is
[INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md), where the **Native**
column shows `yes` or `—`.

Supported:

* lifecycle: `nop`, `return`, `stop`
* arithmetic: `sum`, `sum!`, `diff`, `diff!`, `compare!`, `mul!`, `div.u`,
  `div.s`
* bitwise: `and`, `and!`, `or`, `or!`, `xor`, `xor!`
* movement: `move`
* flags and ordering: `flag.direction`, `fence`
* atomics: `compare.swap`, `exchange`
* control flow: `jump`, `if.zero`, `if.notzero`, `if.carry`, `if.nocarry`,
  `if.sign`, `if.notsign`, `if.overflow`, `if.nooverflow`, `if.even`, `if.odd`,
  `call`, `tailcall`

**Not supported — 61 instructions.** Most importantly:

* **All 16 memory instructions** (`load.*`, `store.*`, `load.acquire.*`,
  `store.release.*`). A native program cannot touch memory at all.
* **All exceptions** (`catch`, `uncatch`, `raise`) and `break`.
* **All 11 `sys.*` services.** This is deliberate: a native executable must
  never acquire a capability the interpreter would refuse.
* **Widening and selection**: `extend.s`, `extend.z`, `select`.
* **Part of the arithmetic set**: `mul` (flag-preserving), `checked.sum`, `neg`,
  `neg!`, `inc`, `inc!`, `dec`, `dec!`, `not`, `not!`, `sumc`, `sumc!`, `diffc`,
  `diffc!`.
* **All shifts, rotates and bit operations** (`shift.*`, `rotate.*`, `bit.*`).
* `address`.

The full list is the “Interpreter-only instructions” line at the end of
[INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md).

## 2. Nothing falls back silently

Requesting a native tier for an image outside the subset **fails**; it does not
fall back to the interpreter.

```
$ +a run program.pa --native
error: this image cannot run natively; it uses 1 instruction(s) outside the native subset: load.u64
       run it without --native (interpreter) or see manual/JIT-AOT.md
```

Exit status is `2`. The same check runs for `+a build --native`,
`+a compile-native`, and the JIT (`+a run --jit` reports
`` `load.u64` (opcode 0x0043) is not supported by the native backends ``).
`native_refusal_names_the_unsupported_instruction` and
`cli_refuses_native_runs_that_need_the_interpreter` pin both the message and the
exit status. `--jit` and `--native` together are rejected as contradictory.

## 3. What the backends guarantee when they *do* accept an image

Native code is expected to behave exactly like the interpreter for the
instructions it lowers. The differential tests compare, for every accepted
program:

* the `stop` status, at full 64-bit width for the JIT and 8-bit for AOT (a
  process exit status is 8 bits);
* flag-visible outcomes (conditional branches, `select`-free programs);
* division overflow and division by zero;
* call/return nesting, including functions with several `return`s.

Tests: `execution_tiers_agree_on_a_representative_matrix` and
`execution_tiers_agree_on_calls_and_branches`.

## 4. Native ABI

The generated function is a normal SysV x86-64 function taking no arguments and
returning `u64`:

* **Registers** live in the native frame at `-8 * (index + 1)` from `%rbp`;
  32 slots, initialised to zero.
* **`FLAGS`** live at `%rbp-264` using the same bit positions as the
  interpreter (`Z=0, C=1, S=2, O=3, P=4, D=5`).
* **`stop`** returns `R0` in `%rax` through `.Lexit`.
* **`return`** is `retq`. The entry code pushes a sentinel `leaq .Lexit(%rip)`
  first, so a top-level `return` ends the program exactly like the interpreter's
  empty call stack.
* **`call`** is `callq`, so nesting uses the host stack — the reason a single
  frame slot could not model recursion.

### 4.1 Faults

A native program cannot return an error the way the interpreter can, so faults
are published through an exported `.bss` word:

* symbol `ascension_fault_code`, 8 bytes, always zero unless a fault occurred;
* the frame records the fault at `%rbp-280` and its code at `%rbp-288`;
* `.Lexit` copies the code into the global via the GOT and returns
  `NATIVE_FAULT_EXIT` (70) from the generated function;
* as a standalone executable the process exits with status `70`.

The JIT reads the global back after the call and converts a non-zero code into a
`JitError`; the CLI converts executable status `70` into
`error: runtime fault: …`. Implemented codes: `1` = divide by zero. Because the
only reachable native fault is division, the native tiers are a *subset*, not a
reimplementation, of the AVM's fault model.

## 5. Reproducibility

* Bytecode is byte-for-byte reproducible.
* Native output is **not**: the assembly text is deterministic, but the
  assembled binary depends on the host `cc`, linker and libc. Treat native
  binaries as build artifacts, not as distributable release objects.

## 6. Requirements and platforms

* `cc` must be on `PATH`; the backends invoke it with `-x assembler -`.
* The JIT requires Linux x86-64 (it uses `dlopen`/`dlsym`), and is compiled out
  elsewhere.
* The AOT backend emits x86-64 SysV assembly and targets Linux x86-64.
* One entry function, function ID 0, is required. Multi-function images cannot
  be lowered by the native backends.

## 7. Known defects in the native tiers

* Only the instructions in §1 are available; a native program is an *integer and
  control-flow* program.
* There is no register allocator: every value lives in its frame slot, so native
  code is not faster than the interpreter on memory-heavy code (which it cannot
  run anyway).
* Fault reporting covers division by zero only; other native-reachable faults
  cannot arise because the instructions that would cause them are not lowered.
