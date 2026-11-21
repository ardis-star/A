# +A Core Documentation

**Edition:** ASCENSION +A Core V1.1.0

Public, implementation-accurate documentation for the +A toolchain. Every
document describes what the code in `rust/src` actually does. Where a claim can
be checked mechanically, a test checks it — those tests are named in the
document so a reader can run them.

## Start here

| Document | Read it for |
|---|---|
| [SPECIFICATION.md](SPECIFICATION.md) | The language: syntax, registers, flags, arithmetic, control flow, exceptions. |
| [INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md) | All 97 instructions, opcodes, operand forms, and which tiers support them. |
| [CLI.md](CLI.md) | Every `+a` command, option, output format, and exit status. |
| [BUILD-AND-INSTALL.md](BUILD-AND-INSTALL.md) | Building, installing, testing, and troubleshooting. |

## Architecture and internals

| Document | Read it for |
|---|---|
| [REGISTERS.md](REGISTERS.md) | Register file, sub-register views, read/write rules, flag layout. |
| [MEMORY-MODEL.md](MEMORY-MODEL.md) | Why there is no implicit memory; regions, access checks, endianness, atomics. |
| [BYTECODE-FORMAT.md](BYTECODE-FORMAT.md) | The `.avm` container, byte for byte. |
| [VERIFICATION.md](VERIFICATION.md) | Every verification rule, in the order they run, with the exact messages. |
| [AVM.md](AVM.md) | The interpreter: initial state, faults, steps, resumption, exceptions. |
| [JIT-AOT.md](JIT-AOT.md) | The native tiers: the 36-instruction subset, the ABI, fault publication, limits. |

## Design and status

| Document | Read it for |
|---|---|
| [SYS-ARCHITECTURE.md](SYS-ARCHITECTURE.md) | Design (not implementation) of the `sys.*` capability boundary. |
| [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) | The honest list of what V1.1.0 cannot do. |

## Reading order for a new contributor

1. `README.md` at the repository root — what the project is.
2. [SPECIFICATION.md](SPECIFICATION.md) — the language.
3. [INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md) — the instruction set.
4. [BYTECODE-FORMAT.md](BYTECODE-FORMAT.md) then [VERIFICATION.md](VERIFICATION.md)
   — how source becomes something executable.
5. [AVM.md](AVM.md) — how it runs.
6. [JIT-AOT.md](JIT-AOT.md) and [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) —
   what is not possible yet.

## Conventions used in these documents

* **Normative** statements describe the implementation. If code and prose
  disagree, the code is authoritative and the prose is a bug.
* “Interpreter-only” means the JIT and AOT backends refuse the instruction with a
  named error; it never means “quietly interpreted”.
* Counts (97 instructions, 36 native, 161 tests) are from the V1.1.0 tree and are
  re-derived by tests where possible.

## Keeping these documents true

| Claim | Checked by |
|---|---|
| The instruction reference lists every mnemonic with the right opcode and native status | `instruction_reference_covers_every_defined_mnemonic` |
| The declared native subset matches the emitter | `native_opcode_table_matches_the_emitter` |
| Memory semantics match [MEMORY-MODEL.md](MEMORY-MODEL.md) | `memory_semantics_match_the_documented_rules` |
| Bytecode is reproducible | `repeated_assembly_is_byte_for_byte_identical` |
| Tiers agree | `execution_tiers_agree_on_a_representative_matrix`, `execution_tiers_agree_on_calls_and_branches` |
| Every `sys.*` service is gated | `every_system_service_requires_a_capability` |

All of these live in `rust/tests/` and run with
`cargo test --lib --tests`. If you change behaviour, change the document and the
test in the same commit.
