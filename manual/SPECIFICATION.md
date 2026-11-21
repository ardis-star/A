# +A Language Specification

**Edition:** ASCENSION +A Core V1.1.0
**Status:** normative for the V1 toolchain in this repository.

This document describes the +A assembly language as the reference implementation
actually accepts it. Where prose and code disagree, the code is authoritative;
please report the discrepancy rather than relying on the prose.

---

## 1. Scope

+A is a **register-based assembly language** for the ASCENSION virtual
architecture. It has no expressions, no types beyond register widths, and no
control constructs beyond labels, branches, and calls. A `.pa` source file is
assembled into a portable bytecode image (`.avm`) that can be:

* interpreted by the AVM,
* compiled to x86-64 at runtime (JIT), or
* compiled ahead of time to a standalone Linux ELF executable.

The language, the bytecode format, and the verifier are specified here. The
execution tiers are specified in [manual/AVM.md](AVM.md) and
[manual/JIT-AOT.md](JIT-AOT.md).

## 2. Source form

* Source files use the **`.pa`** extension. The legacy `.a` extension is still
  accepted for source files; `.ma` is explicitly rejected because that suffix
  belongs to the unrelated `-A` project.
* Files are UTF-8. Line endings may be LF or CRLF; CRLF is normalised to LF
  before lexing, so line and column numbers match the file as written.
* A statement occupies one line. Blank lines are ignored.
* `;;` starts a comment that runs to the end of the line. There is no
  block-comment form.

## 3. Lexical structure

| Token | Form | Notes |
|---|---|---|
| Identifier | `[A-Za-z_][A-Za-z0-9_.!]*` | Mnemonics and labels. `.` and `!` may appear **inside** an identifier, which is how `sum!`, `if.zero`, and `sys.port.read` are single tokens. |
| Number | decimal, `0x` hex, `0b` binary | `_` is allowed as a digit separator anywhere after the prefix, including immediately (`0x_FF`, `0b_1`). A leading `-` makes it negative. |
| Label | `name::` | Declares a label at the current instruction position. |
| `<-` | arrow | Data flow into a destination. |
| `->` | branch | Control transfer to a label. |
| `<->` | swap | Exchange operator. |
| `[` `]` `,` `+` | punctuation | Memory operands and operand separators. |

Number literals are **not** general-purpose integers:

* A decimal literal with a leading zero and another non-zero digit (`007`) is
  rejected, rather than being read as octal.
* A prefixed literal whose first digit is not valid for its radix
  (`0xG`, `0b2`) does not lex as a number; the parser then reports
  `expected end of line`.
* Values are range-checked against the width of the instruction's register
  operand, see §6.

## 4. Program structure

```
unit arithmetic          ;; optional, at most once
export main              ;; optional, at most once in V1

main::
    move R0.q <- 100
    sum! R0.q <- R0.q, 25
    stop
```

A program is a flat instruction stream with labels. There are no nested
functions: every label is an address in the same stream, and `call`/`return`
operate on that stream.

### 4.1 `unit`

`unit <identifier>` names the program. It is optional, may appear at most once,
and has no effect on the emitted bytecode.

### 4.2 `export`

`export <name>` (optionally written `export <name>::`) marks a label as the
entry point. In V1:

* At most **one** export is permitted.
* The exported label must be at instruction zero. An interior label cannot be
  exported.
* The entry-function ID in the image header is `0` when an export is present and
  `0xFFFFFFFF` (library image) when it is not.

## 5. Registers

32 general-purpose registers `R0`–`R31`, each 64 bits wide. A register name may
carry a **view** suffix selecting a sub-register width:

| Suffix | Width | Byte offset in slot |
|---|---|---|
| `.b` | 8 bits | 0 |
| `.w` | 16 bits | 0 |
| `.d` | 32 bits | 0 |
| `.q` (default) | 64 bits | 0 |

`R7` and `R7.q` are the same operand. View rules:

* **Writes** to `.b` and `.w` preserve the register's upper bits. A write to
  `.d` **replaces the whole slot** and zero-extends. A write to `.q` replaces
  the slot.
* **Reads** return the sub-register value zero-extended to 64 bits.
* `SP`, `PC`, `FP`, and `FLAGS` are architectural state, **not** addressable
  registers. Writing `SP` as an operand is a parse error.
* Within one instruction, all register operands must have the same view, except
  `exchange` (full `R`n` only) and `extend.s`/`extend.z` (source narrower than
  destination).
* The register index must be canonical: `R00` and `R01` are rejected, and `R32`
  and above are out of range.

## 6. Operand forms and immediates

| Form | Example | Where |
|---|---|---|
| Register | `R0.q` | most operands |
| Immediate | `42`, `-1`, `0x1122_3344` | sources |
| Memory | `[R1.q + 8]`, `[R1.q - 8]`, `[R1.q]` | load/store destinations and sources |
| Label reference | `-> loop` | branches and calls |
| Predicate | `zero`, `notzero`, … | `select` |
| Symbol | `main` | `address` |

Registered memory addresses use a **full GPR** base plus a signed 32-bit
displacement; a sub-register base is a parse error, and a displacement outside
`[-2^31, 2^31-1]` is rejected.

Every immediate is checked against the **operation width** — the width of the
instruction's register operand. A `.b` operation accepts `-128..=255`, `.w`
accepts `-32768..=65535`, `.d` accepts `-2^31..=2^32-1`, `.q` accepts
`-2^63..=2^64-1`. Out-of-range values are a parse error, never a silent
truncation.

## 7. Instructions

A complete, implementation-derived reference is in
[manual/INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md). The syntax families
are:

| Family | Form | Example |
|---|---|---|
| No operands | `mnemonic` | `nop`, `return`, `stop`, `break`, `uncatch` |
| Destination ← source | `mnemonic dest <- src [, src]` | `move R0.q <- R1.q`, `sum! R0.q <- R1.q, 2` |
| Two operands | `mnemonic a, b` | `compare! R0.q, 42` |
| Two, swappable | `mnemonic a <-> b` | `exchange R0 <-> R1` |
| Two destinations | `mnemonic dq, dm <- a, b` | `div.u R2.q, R3.q <- R0.q, 7` |
| Memory load | `load.uN dest <- [base + off]` | `load.u32 R0.d <- [R1.q + 4]` |
| Memory store | `store.uN [base + off] <- src` | `store.u8 [R1.q] <- R0.b` |
| Branch | `mnemonic -> label` | `if.zero -> done` |
| Select | `select dest <- pred, a, b` | `select R0.q <- carry, 1, 2` |
| Address | `address dest <- symbol` | `address R0.q <- main` |
| Compare-and-swap | `compare.swap dest <- [base+off], expected, desired` | see reference |
| Mode operand | `flag.direction <- forward` | `fence full`, `flag.direction <- backward` |
| System service | `sys.<service>` | `sys.port.write` |

Mnemonics ending in `!` update the architectural flags. The same operation
without `!` leaves the flags untouched. `compare!` and `bit.test!` produce flags
without writing a destination.

## 8. Flags and conditions

Six flags live in `FLAGS`: **Z**ero, **S**ign, **C**arry, **O**verflow,
**P**arity (even), and **D**irection.

* A flag-setting instruction recomputes Z, S, C, O, and P from its result. The D
  flag is preserved.
* `flag.direction` sets or clears D and preserves all other flags.
* `select` and the conditional branches read flags and never modify them.
* Carry-in forms (`sumc`, `diffc`) consume C as an input.
* P is the even parity of the **low byte** of the result, matching x86 `PF`.
* The division family (`div.u`, `div.s`) does **not** modify flags.

Branch/select conditions: `zero`, `notzero`, `carry`, `nocarry`, `sign`,
`notsign`, `overflow`, `nooverflow`, `even`, `odd`.

## 9. Arithmetic semantics

All arithmetic is performed at the instruction's width and masked to it. The
following are normative:

* **Addition/subtraction** wrap at the operation width. C is set when the
  unsigned result overflows the width; O is set on signed overflow.
* **Multiplication** is truncating: the low *width* bits of the product are
  kept, C and O are cleared.
* **`checked.sum`** faults with `INTEGER_OVERFLOW` instead of wrapping, and does
  **not** commit the destination.
* **`neg`** of the most negative value sets O. **`inc`**/`dec`** set O on signed
  overflow across the width boundary.
* **Divisions** produce a quotient *and* a remainder into two destinations.
  Rounding is toward zero (`-7 / 2` gives quotient `-3`, remainder `-1`).
  A zero divisor faults with `DIVIDE_BY_ZERO`. The one unrepresentable signed
  quotient (most negative value divided by `-1`) faults with
  `INTEGER_OVERFLOW`. Divisions do not set flags.
* **Shifts** use the low bits of the count. `shift.l`/`shift.r`/`shift.ar`
  shift by `count mod width`; a count of zero is a no-op and clears C. `carry`
  reports the last bit shifted out. Rotates by `count mod width`; a rotate by
  zero is a no-op.
* **`bit.set`/`bit.clear`/`bit.test!`** use `index mod width`.
* **`compare! a, b`** is `a - b` for flag purposes only.

## 10. Memory model

The AVM has no implicit memory. A program can only touch regions the embedder
mapped (see [manual/AVM.md](AVM.md)). Bytecode has no notion of a stack or heap;
those are conventions, and V1 imposes no stack discipline. Compilers that need
one must establish it with `sys.region.map` in a privileged context.

Access rules enforced at run time:

* The access width and the register view must match: `load.u32` into `.d`,
  `store.u8` from `.b`, and so on. A mismatch is an assembly error.
* Loads write through the destination's view, so a narrow load preserves the
  register's upper bits; there is no widening load.
* The whole access must lie inside exactly one mapped region.
* Loads require the region to be readable; stores require it to be writable.
* `*.acquire`/`*.release` accesses additionally require the region to be
  `shared` and `atomic`, and require natural alignment; a violation is an
  `ALIGNMENT` or `MEMORY_ACCESS` fault.
* Byte order is little-endian.
* Ordinary unaligned access is permitted; `*.acquire`/`*.release` require
  natural alignment.

## 11. Control flow

* `jump -> label` transfers unconditionally.
* `if.<cond> -> label` transfers when the condition holds.
* `call -> label` pushes the return point and transfers; `return` pops it. With
  an empty call stack `return` ends the program with `returned`.
* `tailcall -> label` transfers without pushing.
* Blocks are whole instructions. A label between two statements always starts a
  new block, and instruction zero is always a block boundary.
* A label at the very end of the program, with no instruction after it, cannot
  be a branch target.

## 12. Exceptions

`catch -> handler` pushes a handler; `uncatch` pops one; `raise code` transfers
to the innermost handler of the **current function** and puts `code` in `R0`.

* A handler is scoped to the function that registered it. A `raise` never jumps
  to a handler registered in another function.
* `raise` with no handler registered by the current function faults with
  `INVALID_HANDLER`.
* The verifier rejects an image whose instruction stream pops more handlers than
  it pushes within a function (`uncatch` underflow).

## 13. Errors

Assembly errors are reported as `path:line:column: message`. The toolchain
distinguishes:

* lexing errors (`unexpected character`),
* parsing errors (`expected ARROW`, `does not fit 8 bits`, …),
* encoding errors (`undefined label`, `exported symbol ... is not a label`),
* verification errors (see [manual/VERIFICATION.md](VERIFICATION.md)),
* runtime faults (see [manual/AVM.md](AVM.md)).

## 14. Determinism

Assembling the same source twice produces byte-identical images. The assembler
uses ordered maps throughout, so label ordering does not leak into output. This
is covered by tests, including exact golden images.

## 15. Compatibility notes for V1.1.0

The following behaviours changed in V1.1.0 and are documented in
[CHANGELOG.md](../CHANGELOG.md):

* `div.s` of the most negative value by `-1` now reports `INTEGER_OVERFLOW`
  rather than `DIVIDE_BY_ZERO`.
* `address` resolves through the symbol table and yields a **VM address**;
  native backends reject it.
* `raise` no longer transfers to a handler registered in another function.
