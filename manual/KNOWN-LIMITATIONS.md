# Known Limitations — +A Core V1.1.0

**Edition:** ASCENSION +A Core V1.1.0

This is the honest list. Everything here is a deliberate boundary of the current
release or a documented defect; nothing on this list is hidden behind a silent
fallback.

## 1. Language and assembler

| Limitation | Consequence | Workaround |
|---|---|---|
| Exactly **one** exported label, and it must label instruction zero | An image has one function | `call`/`return`/`tailcall` within that function |
| No parameter or result mechanism | Functions communicate through registers the author picks | Document the convention in the program |
| No macros, includes, or conditional assembly | Source is a flat instruction stream | Generate source externally |
| No stack, no frame, no calling convention | Recursive *data* structures need manual memory management | Use `sys.region.map` (privileged, and not implemented — see §4) |
| Labels must have at least one instruction after them | A label on the final line cannot be a branch target | Add a trailing `nop` |
| Comments are `;;` only, no block comments | — | — |
| Immediates are range-checked to the operation width | A `.b` write of `300` is an assembly error | Use a wider view or mask first |

## 2. Execution tiers

| Limitation | Detail |
|---|---|
| **The native tiers support 36 of 97 instructions** | No memory access, no exceptions, no `sys.*`, no shifts/rotates/bit ops, no `extend`/`select`, and part of the arithmetic set is missing. The authoritative list is [INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md). |
| Native binaries are not reproducible | Linking is delegated to the host `cc`. |
| JIT is Linux x86-64 only | `run_jit` returns a diagnosable error elsewhere. |
| AOT emits x86-64 SysV assembly | Other targets are unsupported. |
| Native requires one entry function with ID 0 | Multi-function images cannot be lowered. |
| Native fault reporting covers division by zero only | Other AVM faults are unreachable natively because their instructions are not lowered. |
| Native `stop` status is truncated to 8 bits in AOT | A process exit status is 8 bits. The JIT reports the full 64-bit value. |
| No register allocator | Native code keeps every value in its frame slot. |

## 3. Memory

| Limitation | Detail |
|---|---|
| The CLI maps no memory | Any memory-touching program faults with `MEMORY_ACCESS` when run via `+a`. Region mapping is an embedder API. |
| `DATA` sections are not mapped into the address space | There is no instruction that reads them. |
| No allocator | Regions come from the host. |
| Unaligned ordinary access is allowed | Both tiers are x86-64; programs that need portability should stay aligned. |
| Narrow loads are not widening | `load.u8 R0.b` preserves `R0`'s upper bits; clear the register first to widen. |

## 4. System services

| Limitation | Detail |
|---|---|
| **No `sys.*` service is implemented** | Every service returns `UNSUPPORTED_SERVICE` once privilege and capability checks pass. The boundary is enforced; the table is empty. See [SYS-ARCHITECTURE.md](SYS-ARCHITECTURE.md). |
| No threads, interrupts, timers, or preemption | The AVM is single-threaded and deterministic. |
| No capability escalation | Capabilities are fixed when the machine is constructed. |

## 5. Verification

| Limitation | Detail |
|---|---|
| Verification is structural, not a proof | It guarantees well-formedness and determinism, not that the program terminates or that its accesses will succeed. |
| Branches resolve only within the current function's block map | A cross-function branch fails to verify. Since `+a` emits only one function, this cannot arise from toolchain output. |
| Minor version 0 images with unresolved control flow are refused | `verify_image` reports `unresolved control targets` rather than guessing. |
| `inspect` is weaker than `verify` | It performs the structural half only, deliberately. |

## 6. Toolchain and platform

| Limitation | Detail |
|---|---|
| No `rustdoc` in every toolchain | Doc-tests cannot be run; use `cargo test --lib --tests`. |
| `cc` required for JIT/AOT | Tests that use those tiers fail rather than skip when `cc` is absent. |
| Only Linux x86-64 is validated | Other platforms are expected to build the interpreter but are untested. |
| No release automation | V1.1.0 is tagged and documented; no binary release is produced by CI. |

## 7. Documentation

* The public docs live in `manual/`. They are written against the implementation
  and, where machine-checkable, verified by tests
  (`instruction_reference_covers_every_defined_mnemonic`,
  `native_opcode_table_matches_the_emitter`,
  `memory_semantics_match_the_documented_rules`).
* Anything not covered here is not a promise. When prose and code disagree, the
  code is authoritative, and the disagreement is a documentation bug to report.

## 8. Deliberately out of scope for Core

These belong to "+A Foundation" and are not started:

* an implemented service table, allocator, and resource quotas;
* a calling convention and stack discipline spanning more than one function;
* multi-function images and linking;
* an object/archive format, or a symbolic debugger;
* preemptive scheduling, virtual memory, or device drivers;
* JIT/AOT support for the remaining 61 instructions.
