# JIT and AOT Backends: Capabilities and Limitations

**Edition:** ASCENSION +A Core V2.0.0

The toolchain has three execution tiers. This document states precisely what the
two native tiers do, what they cannot do, and how a refusal is reported.

| Tier | Command | Mechanism |
|---|---|---|
| Interpreter | `+a run <file>` (default), `+a <source.pa>` | `ascension_asm::avm::Avm` |
| JIT | `+a run <file> --jit`, `+a run-jit <image>` | emits x86-64 assembly, assembles and links a shared object with `cc`, `dlopen`s it, calls the entry point |
| AOT | `+a run <file> --native`, `+a build --native`, `+a compile-native` | emits x86-64 assembly, assembles and links a standalone ELF executable with `cc` |

## 1. The native subset

The native emitter lowers **51 of the 97** defined instructions. The safe JIT
API and CLI refuse the 17 memory-accessing operations (`compare.swap` and
the 16 loads/stores); only the explicitly unsafe library entry point may run
them in-process. AOT supports the full emitter subset. The subset is
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
* memory: `load.u8`, `load.u16`, `load.u32`, `load.u64`, `store.u8`,
  `store.u16`, `store.u32`, `store.u64`, `load.acquire.u8` … `u64`,
  `store.release.u8` … `u64`
* flags and ordering: `flag.direction`, `fence`
* atomics: `compare.swap`, `exchange`
* control flow: `jump`, `if.zero`, `if.notzero`, `if.carry`, `if.nocarry`,
  `if.sign`, `if.notsign`, `if.overflow`, `if.nooverflow`, `if.even`, `if.odd`,
  `call`, `tailcall`

**Not supported — 46 instructions.** Most importantly:

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

### 1.1 Native memory semantics

AOT and unsafe JIT memory instructions access the **host address space directly**, like C:
there are no region checks, no permission checks, and no bounds checking. An
address that is not mapped faults with a host signal (`SIGSEGV`), not with a
VM fault. This is the deliberate difference from the AVM, which is sandboxed
(see [MEMORY-MODEL.md](MEMORY-MODEL.md)): native code is trusted code.

Consequences:

* ordinary unaligned access works, exactly as in the AVM;
* atomic accesses (`load.acquire.*`, `store.release.*`, `compare.swap`) should
  be naturally aligned, as in the AVM — the backend does not check, but unaligned accesses may fault or incur costly split locks; the AVM rejects them;
* a hosted native program must only touch memory its process owns; a
  freestanding program (§8) touches addresses in the active address space; virtual-to-physical mapping is controlled by the loader.

## 2. Nothing falls back silently

Requesting a native tier for an image outside the subset **fails**; it does not
fall back to the interpreter.

```
$ +a run program.pa --native
error: this image cannot run natively; it uses 1 instruction(s) outside the native subset: select
       run it without --native (interpreter) or see manual/JIT-AOT.md
```

Exit status is `2`. The same check runs for `+a build --native`,
`+a compile-native`, and the JIT (`+a run --jit` reports
`` `select` (opcode 0x0013) is not supported by the native backends ``).
`native_refusal_names_the_unsupported_instruction` and
`cli_refuses_native_runs_that_need_the_interpreter` pin both the message and the
exit status. `--jit` and `--native` together are rejected as contradictory.

## 3. What the backends guarantee when they *do* accept an image

For valid native addresses and within the supported subset, native code is
intended to match the interpreter; memory faults, resource limits, and atomic
validation differ as noted here. The differential tests compare representative programs for:

* the `stop` status, at full 64-bit width for the JIT and 8-bit for AOT (a
  process exit status is 8 bits);
* flag-visible outcomes (conditional branches, `select`-free programs);
* division overflow and division by zero;
* call/return nesting, including functions with several `return`s;
* memory contents for `load`/`store` programs (JIT, in-process).

Tests: `execution_tiers_agree_on_a_representative_matrix`,
`execution_tiers_agree_on_calls_and_branches`, `native_memory_matches_the_interpreter`,
`native_narrow_and_ordered_memory_matches`, `mul_flag_semantics_agree_across_tiers`.

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
`JitError`. For `+a run --native`, the CLI builds a special executable that
traps on VM faults, so a normal status of 70 is not misdiagnosed. A signal
termination is reported as a runtime fault; a system may produce a core dump.
A standalone executable built via `build --native` or `compile-native` still
uses exit status 70 for a fault, indistinguishable from a normal stop of 70. Implemented codes: `1` = divide by zero. Because the
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

## 7. Native-tier limitations

* Only the instructions in §1 are available; a native program is an *integer,
  memory, and control-flow* program.
* There is no register allocator: every value lives in its frame slot, so native
  code is not faster than the interpreter on memory-heavy code.
* Fault reporting covers division by zero only; other native-reachable faults
  cannot arise because the instructions that would cause them are not lowered.
  An invalid memory address crashes the process with a host signal instead
  (see §1.1).

## 8. Freestanding output (bootloaders and kernels)

Core can emit **freestanding** native code that needs no operating system and
no AVM runtime: a static ELF with a `_start` entry, no libc, and control of
the text virtual address. It requires an x86-64 long-mode loader and a valid stack. A minimal +A kernel
was boot-tested in QEMU using an external Multiboot bootstrap (see
[V2-ACCEPTANCE.md](V2-ACCEPTANCE.md)); it is not a complete boot image. +A
does not expose arbitrary machine instructions, direct hardware-register or
stack control, or calls from +A to externally linked routines; the external
bootstrap supplies initial machine setup. Further interoperability is outside
Core V2.0.0.

```console
$ +a compile-native program.avm -o boot.bin --freestanding --load-address 0x100000
$ +a compile-native program.avm -o prog.o --object
```

* `--freestanding` links with `-nostdlib -static -Wl,-e,_start`. The image
  has no `PT_INTERP` segment and no dynamic dependencies; it can be loaded at
  a fixed address by firmware, a bootloader, or a kernel loader.
* `--load-address <addr>` (hex `0x…` or decimal) is passed as `-Ttext`, so
  the caller chooses the text virtual address (for example `0x100000`). The
  output is an ELF, not a raw boot sector; the loader must honor its segments. It requires `--freestanding`.
* `--object` assembles a relocatable object (ET_REL) exporting `_start`
  without linking, so the caller's own build system can link it with other
  objects or a custom linker script.
* The entry expects x86-64 long mode and a valid stack in `%rsp`, supplied
  by a compatible loader; the program uses 304 bytes of frame plus call depth. Registers
  start at zero, exactly as in the hosted ABI.
* `stop` (and a top-level `return`) **halts the CPU** (`hlt` loop) with the
  status in `%rax`; a runtime fault **traps** (`ud2`) with the fault code in
  `%rax`. There is no OS to return to, so there is no exit status — the
  loader inspects `%rax`.
* Memory instructions touch the addresses the loader mapped, with the same
  direct semantics as §1.1. There is no sandbox in freestanding mode: the
  image *is* the privileged code.

The library API is `emit_assembly_for(image, NativeTarget::Freestanding)`,
`compile_freestanding(image, output, load_address)`, and
`compile_object(image, output)` in `rust/src/native.rs`. Freestanding output
is covered by `freestanding_output_is_a_static_start_image`,
`object_output_is_relocatable`, and `test_cli_freestanding_and_object_builds`.

**Security:** The safe `run_jit64` / `run_jit` APIs and CLI reject direct memory
operations. `unsafe run_jit64_unchecked` allows them only if the caller ensures
that all accesses satisfy its safety contract. Neither verification nor the JIT
provides resource isolation: untrusted programs may loop or exhaust the native
stack. Use the AVM with appropriate limits for untrusted bytecode. This security boundary does not imply resource isolation.
