# Changelog

All notable changes to the ASCENSION +A toolchain will be documented in this file.

## [2.0.0] - Unreleased

**+A Core V2.0.0 — finalized, not yet published.** The assembler, bytecode,
verifier, AVM, documented JIT/AOT subset, CLI, installation and freestanding
output have been validated within their stated scope. Arbitrary machine-level
interop and complete OS interfaces are outside Core V2.0.0; the documented
freestanding path uses external boot infrastructure. The bytecode version is
unchanged, but stricter verification can reject previously accepted malformed
images. Compatibility has been tested against representative V1.1.0 fixtures,
not exhaustively against all prior artifacts.

### Added

- **Native memory access.** All 16 memory instructions (`load.*`, `store.*`,
  `load.acquire.*`, `store.release.*`) are now lowered by the JIT and AOT
  backends. The native subset grows from 36 to 51 of 97 instructions. Native
  memory is direct host memory (no region checks); an invalid address crashes
  with a host signal or corrupt host memory. Safe JIT entry points and the CLI
  reject memory instructions; `unsafe run_jit64_unchecked` exposes them to
  callers responsible for host-memory safety. See `manual/JIT-AOT.md` §1.1.
  *Tests:* `native_memory_matches_the_interpreter`,
  `native_narrow_and_ordered_memory_matches`.
- **Freestanding native output.** `+a compile-native` and `+a build --native`
  accept `--freestanding` (static `_start` ELF, no libc, optional
  `--load-address`), `--object` (relocatable ET_REL object), and
  `--load-address <addr>` (`-Ttext`). `stop` halts with the status in `%rax`;
  a fault traps with the fault code in `%rax`. Library API:
  `native::emit_assembly_for`, `native::compile_freestanding`,
  `native::compile_object`, `native::NativeTarget`. See
  `manual/JIT-AOT.md` §8.
  *Tests:* `freestanding_output_is_a_static_start_image`,
  `object_output_is_relocatable`, `test_cli_freestanding_and_object_builds`.

### Fixed

1. **`NATIVE_OPCODE_RANGES` claimed `address` was native.** The range
   `0x000A..=0x000D` included `0x000B`, but the emitter rejected `address`,
   so `unsupported_instructions` never listed it and the refusal surfaced as
   a late emitter error instead of the named upfront refusal. The range now
   excludes `0x000B`, and the instruction reference agrees.
   *Test:* `instruction_reference_covers_every_defined_mnemonic`,
   `native_refusal_names_the_unsupported_instruction`.
2. **The verifier accepted out-of-range mode operands.** A hand-crafted image
   could carry `fence` mode 5 or `flag.direction` mode 2; the AVM accepted
   them while the native backend refused, so tiers disagreed. The verifier
   now rejects them with `mode operand is outside its defined range`.
   *Tests:* `verifier_fence_mode_outside_defined_range_rejected`,
   `verifier_flag_direction_mode_outside_defined_range_rejected`.
3. **The verifier accepted `address` operands naming missing symbols.** An
   out-of-range symbol index passed verification and faulted only at run
   time. It is now rejected with `symbol index is outside SYMS section`.
   *Test:* `verifier_symbol_index_outside_syms_rejected`.
4. **The library lexer rejected CRLF.** `assemble()` failed on `\r\n` line
   endings; only the CLI normalised them. The lexer now treats LF, CRLF, and
   CR as line endings, so the library and CLI agree.
   *Test:* `crlf_source_assembles_via_library`.
5. **Direct `+a <source.pa>` silently ignored contradictory flags.**
   `--jit --native` together ran the JIT without complaint, and `-o` combined
   with `--native`/`--jit` wrote bytecode while ignoring the tier. Both are
   now errors, matching `run`.
   *Test:* `test_cli_direct_mode_rejects_contradictory_flags`.
6. **Clippy `-D warnings` failed.** Four `uninlined_format_args` lints in
   `bytecode.rs` and `cli.rs` are fixed, so the documented lint command is
   clean again.

7. **Narrow `compare.swap` was broken.** The AVM attempted to convert a short
   slice into an eight-byte array, and native code used a 32-bit operation for
   byte/word widths, lacked the `lock` prefix and clobbered a callee-saved
   register. Both tiers now use the requested width; native uses locked CAS
   and a caller-saved register. Regression: `compare_swap_all_widths_preserves_adjacent_bytes_and_matches_jit`.
8. **Native direction flag handling violated the host ABI.** It used `std`/`cld`
   rather than updating the virtual FLAGS slot. It now updates virtual state
   without leaving the host direction flag set.
9. **A forged `SYMS` count could cause excessive allocation.** The verifier
   bounds the count by the section size before allocation. Exception range
   overlap and block-boundary checks also avoid quadratic scans.
10. **Object output silently ignored `--load-address`.** This combination is
    now rejected; relocation addresses must be chosen at link time.
11. **Safe JIT functions allowed arbitrary host memory access.** They now
    reject memory instructions; trusted callers can explicitly use
    `unsafe run_jit64_unchecked` with its safety requirements.

12. **CLI native runs confused stop status 70 with a fault and ignored signal
    termination.** CLI-run executables now trap VM faults; signals are errors
    and a normal status of 70 is reported normally. Standalone binaries keep
    their existing exit-70 convention.
13. **Large multi-function images triggered quadratic verifier scans.**
    Function ranges are indexed and block records grouped by function. The
    maximum-count stress test verifies 65,536 functions and blocks.

### Changed behaviour (observable; documented, not silent)

| Change | Affects | Rationale |
|---|---|---|
| Native subset is 51 of 97 (was 36) | native workflows | memory access is now lowered; `address` is correctly excluded |
| `META` producer string is `+A assembler 2.0.0` (was `0.1`) | golden images | version synchronisation; content is informational only |
| Out-of-range modes and symbol indices fail verification | hand-crafted images | invalid bytecode is rejected at the gate, not at run time |
| Direct mode rejects `--jit --native` and `-o` with a tier | CLI users | contradictory requests now fail instead of being ignored |

### Test results (executed, V2.0.0)

```
cargo test --locked --offline
tests/assembler.rs   73 passed, 1 ignored
tests/avm.rs         36 passed
tests/cli.rs         14 passed
tests/regressions.rs 36 passed
tests/compatibility.rs 1 passed
tests/verifier.rs    22 passed
                     ────────────────────
                    182 passed, 1 ignored, 0 failed
```

On pinned Rust 1.97.0, doc-tests (zero examples), `cargo fmt -- --check`, and
`cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
passed. A clean-source build and installation passed, and a minimal +A kernel
booted under QEMU 11.0.2; see `manual/V2-ACCEPTANCE.md`.

### Compatibility

- **Bytecode format**: the A+BC major version remains 1 (minor 0 or 1).
  The producer `META` string differs and
  malformed inputs are checked more strictly. Compatibility with
  prior release artifacts has not been exhaustively tested; five V1.1.0
  example images verify and run as regression fixtures.
- **Rust API**: `run_jit` and `run_jit64` now reject memory instructions;
  trusted embeddings can use the new unsafe API. `emit_assembly` retains its signature and
  meaning (hosted target); `emit_assembly_for`, `compile_freestanding`,
  `compile_object`, and `NativeTarget` are new.
- **CLI**: additive, except that contradictory or unsupported requests now
  fail instead of being ignored (see "Changed behaviour").
- **Release tags**: `v1.0.0` and `v1.1.0` are untouched.

## [1.1.0] - 2026-11-21

**+A Core V1.1.0 — correctness release.** Every defect found in the V1.0.0 audit
is fixed, and each one has a permanent regression test. No public API was
removed; the changes below are documented in "Changed behaviour" where a program
could observe a difference.

### Fixed — bugs reproduced, confirmed, and repaired

1. **`native.rs emit_compare_swap` ignored the memory offset.** The atomic
   compare-and-swap lowered to `cmpxchg` against the base register only, so any
   non-zero displacement addressed the wrong word. It now folds the displacement
   into the effective address and uses the correct 32/64-bit `cmpxchg` form.
   *Tests:* `compare_swap_honours_memory_offset`,
   `compare_swap_offset_selects_the_right_word`.
2. **`.d` narrow register writes ignored their source value.** `write_register`
   with view `.d` always staged `%eax`/`%rax` instead of the `source` argument it
   was given. The visible consequence is `div`, whose remainder is staged in
   `%rdx`: both destinations received the quotient and the remainder was silently
   lost. `.d` writes now store the value actually supplied, replace the whole slot
   (zero-extended), and `.b`/`.w` still preserve the upper bits. *Tests:*
   `dword_destination_stores_its_own_value`, `every_register_view_lowers_natively`.
3. **`emit_div` divided with a bare `div`/`idiv` regardless of width.** The
   emitter wrote `div %rcx` / `idiv %rcx` with no size suffix, so the divisor
   width did not match the operand setup and narrow signed division produced a
   different quotient *and* remainder from the interpreter. The backend now
   sign-extends narrow signed operands from their own width, divides with the
   full 64-bit `idivq`/`divq`, and masks both results to the operation width —
   which is exactly the interpreter's rule. *Test:*
   `narrow_signed_division_matches_the_interpreter`.
4. **`+a run image.avm --native` silently ignored `--native`.** The bytecode path
   executed in the interpreter regardless of the flag. All three `--native`
   entry points (source, bytecode-via-`run`, and direct) now route through one
   helper that compiles and runs natively. *Test:*
   `run_bytecode_with_native_flag_builds_native_binary`.
5. **`usage()` omitted implemented commands.** `run-jit`, `compile-native`,
   `verify`, and `inspect` were missing from the usage banner. The banner is now
   complete, and a test parses the command table and asserts the banner lists
   every dispatched command. *Test:* `usage_lists_every_implemented_command`.
6. **`validate_capabilities` accepted trailing bytes.** The `CAPS` length check
   was `expected > data.len()`; an image could append arbitrary data after the
   capability records and still verify. It is now an equality check.
   *Test:* `caps_section_with_trailing_bytes_is_rejected`.
7. **`raise` mishandled handlers across function boundaries.** `raise` popped the
   innermost handler regardless of which function registered it, so a `raise` in
   one function could jump to a handler in another. Handler lookup is now
   function-scoped, dead frames' handlers are discarded during unwinding, and a
   `raise` with no local handler faults with `INVALID_HANDLER`. *Tests:*
   `raise_only_targets_a_handler_in_the_raising_function`,
   `raise_without_a_local_handler_faults`.

### Fixed — defects found during the audit (not in the original report)

8. **`call`/`return` could only model a single return point.** The native backend
   stored one return address in a frame slot, so any function containing a
   `return` before its last instruction looped forever: the second call reused
   the stale slot. The backend now uses the machine stack (`callq`/`retq`) with a
   sentinel return address pushed at entry, so nesting and multiple returns
   behave like the interpreter. *Test:*
   `execution_tiers_agree_on_calls_and_branches`.
9. **`division overflow` was reported as a divide fault.** `div.s` of the most
   negative value by `-1` reported `DIVIDE_BY_ZERO`. It is an overflow, and now
   reports `INTEGER_OVERFLOW` in the interpreter and as a distinct native fault.
   *Tests:* `signed_division_overflow_is_not_a_divide_fault`,
   `native_division_by_zero_reports_a_fault`.
10. **`address` disagreed between tiers.** The interpreter resolved a symbol
    index against the *function table*; the native tiers could not lower it at
    all and failed with a generic message. `address` now resolves through the
    image's `SYMS` section to a **VM address**, and the native backends reject it
    with an explicit, named error. *Test:* `avm_address`.
11. **Native faults were unreachable from the JIT.** Generated code detected a
    division by zero but had no way to report it. Faults now publish a code
    through an exported `ascension_fault_code` word (via the GOT), return
    `NATIVE_FAULT_EXIT` (70), and the JIT and CLI translate that into an error
    instead of a bogus status. *Test:* `native_division_by_zero_reports_a_fault`.

### Added

- **Honest native-subset reporting.** `NATIVE_OPCODE_RANGES` is the single
  declared source of truth for what the native tiers can lower (36 of 97
  instructions). `unsupported_instructions(image)` lists what an image needs that
  the native tiers cannot provide, and `--native`/`--jit`/`compile-native` refuse
  such an image **by name** with exit status 2, pointing at
  `manual/JIT-AOT.md`. No tier silently falls back to another.
  *Tests:* `native_opcode_table_matches_the_emitter`,
  `native_refusal_names_the_unsupported_instruction`,
  `cli_refuses_native_runs_that_need_the_interpreter`.
- **`run_jit64`** returns the full 64-bit `stop` status; `run_jit` remains as a
  compatibility wrapper returning the low 32 bits. `+a run --jit` and
  `run-jit` print `jit stopped: <status>`. *Test:* `jit_reports_the_full_stop_status`.
- **`instructions::mnemonic_for(opcode)`**, so diagnostics can name an
  instruction they refuse; `definition_by_opcode` now derives from it, removing
  a duplicated table.
- **`verifier::Symbol` and `DecodedImage::symbols`**: the `SYMS` section is now
  exposed as a typed, decoded list instead of being validated and discarded.
  `Avm::symbols` and `Avm::code_base` accompany it.
- **`rust-toolchain.toml`** pinning Rust 1.97.0 (plus `rustfmt`, `clippy`) for
  reproducible diagnostics, and a documented release profile (`lto`, `strip`).
- **Public documentation** in `manual/`: language specification, instruction
  reference, register architecture, flags, memory model, bytecode format,
  verification rules, AVM architecture, JIT/AOT limitations, CLI reference,
  build/install, `sys.*` design, and known limitations. Three of these are
  machine-checked against the code.
- **`sys.*` architecture design** (`manual/SYS-ARCHITECTURE.md`): capabilities,
  the privilege predicate, the service surface, and the migration path to
  Foundation. Design only — nothing is implemented.

### Changed behaviour (observable; documented, not silent)

| Change | Affects | Rationale |
|---|---|---|
| `div.s MIN / -1` reports `INTEGER_OVERFLOW` instead of `DIVIDE_BY_ZERO` | programs that divide `INT64_MIN` by `-1` | it is an overflow; the old code was a misclassification |
| `address` yields a **VM address** and resolves through `SYMS` | programs using `address` | the previous result was an in-image offset that native code could not produce |
| `raise` cannot reach a handler in another function | programs that relied on cross-function `raise` | cross-function handlers are unsound: block IDs are function-local |
| `+a run x.avm --native` now actually runs natively | bytecode workflows | the flag was previously ignored |
| Native runs of interpreter-only images now fail instead of silently interpreting | native workflows | honesty: a requested tier must run or report |
| `+a <src> -o <out>` verifies before writing and prints `built: <path>` | scripted builds | previously wrote unverified bytecode silently |
| `+a compile-native` verifies and checks the subset before invoking `cc` | native builds | no partial artifacts from bad input |
| `--jit --native` together is an error | CLI users | the modes are mutually exclusive |
| `cargo` MSRV raised to 1.87 | builders | the code uses `u64::is_multiple_of` |

### Test results (executed, V1.1.0)

```
cargo test --locked --offline --lib --tests
tests/assembler.rs   72 passed, 1 ignored
tests/avm.rs         36 passed
tests/cli.rs          9 passed
tests/regressions.rs 28 passed
tests/verifier.rs    16 passed
                     ────────────────────
                    161 passed, 1 ignored, 0 failed
```

`cargo fmt -- --check` is clean. `cargo clippy --locked --offline --all-targets
--all-features -- -D warnings` is clean. Doc-tests were **not** run: the build
toolchain in use does not include `rustdoc`. This is stated rather than glossed
over.

### Compatibility

- **Bytecode format**: unchanged. Format is still `A+BC` 1.0, and V1.1.0 emits
  and accepts exactly what V1.0.0 did. Images produced by V1.0.0 still verify and
  run.
- **Rust API**: additive only. `run_jit` retains its signature and meaning.
- **CLI**: additive, except that contradictory or unsupported requests now fail
  instead of being ignored (see "Changed behaviour").
- **Release tag**: `v1.0.0` is untouched.

## [1.0.0] - 2026-11-21

### Added
- **Official `.pa` Extension**: Full recognition and validation of `.pa` (Plus A) source files across the entire toolchain.
- **Unified `+a` CLI**: Production-ready command-line interface supporting direct execution (`+a file.pa`), `run`, `build`, `check`, `verify`, and `inspect`.
- **Golden Test Suite**: Bytecode golden verification tests ensuring deterministic binary output.
- **Complete Documentation Suite**: Architectural guides, instruction references, bytecode specifications, runtime guides, and limitation bounds.
- **Installation Scripts**: Automated `install.sh` and Cargo installation targets for `+a`, `plus-a`, and `aa`.

### Changed
- Migrated all examples and internal integration tests to `.pa`.
- Formalized V1.0 release metadata across `Cargo.toml` and CLI headers.

### Verified
- 100% passing test suite (133 tests) covering assembler, bytecode verification, AVM execution, JIT, and native AOT compilation.
- Zero compiler or Clippy warnings with `-D warnings`.
